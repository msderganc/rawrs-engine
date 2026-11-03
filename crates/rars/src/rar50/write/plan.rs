//! Codec settings and execution decisions, resolved before scheduling work.

use super::FilterPolicy;
use crate::codec::rar50::EncodeOptions;
use crate::streaming::preparation::Records;
use crate::{Result, WriterResources};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackReason {
    AutomaticFilterWorkspace { required: u64, limit: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Execution {
    WholeMember,
    Blocks { fallback: FallbackReason },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MemberPlan {
    pub(super) execution: Execution,
    /// Whole-member workspace or one streaming job's workspace, never both.
    pub(super) workspace: u64,
}

pub(super) enum ExecutionPlan {
    Stored,
    Blocks { workspace: u64 },
    IndependentMembers(Records<MemberPlan>),
}

impl ExecutionPlan {
    #[cfg(test)]
    pub(super) fn new(
        settings: &CompressPlan,
        sizes: impl ExactSizeIterator<Item = u64>,
        limit: u64,
    ) -> Self {
        Self::with_resources(settings, sizes, &WriterResources::new(limit)).unwrap()
    }

    pub(super) fn with_resources(
        settings: &CompressPlan,
        sizes: impl ExactSizeIterator<Item = u64>,
        resources: &WriterResources,
    ) -> Result<Self> {
        let limit = resources.memory_limit();
        // Select the requested mode before estimating its active workspace.
        if settings.method == 0 {
            return Ok(Self::Stored);
        }
        let whole_members = !settings.solid
            && (settings.filter_policy != FilterPolicy::None || settings.candidates.len() > 1);
        if !whole_members {
            return Ok(Self::Blocks {
                workspace: streaming_workspace(settings, true),
            });
        }
        Ok(Self::IndependentMembers(Records::collect(
            sizes.map(|size| {
                let required = if size == 0 {
                    0
                } else {
                    whole_member_workspace(size, settings)
                };
                Ok(
                    if (required <= limit && size < whole_member_search_limit(settings))
                        || !matches!(
                            settings.filter_policy,
                            FilterPolicy::Auto | FilterPolicy::Independent
                        )
                    {
                        MemberPlan {
                            execution: Execution::WholeMember,
                            workspace: required,
                        }
                    } else {
                        MemberPlan {
                            execution: Execution::Blocks {
                                fallback: FallbackReason::AutomaticFilterWorkspace {
                                    required,
                                    limit,
                                },
                            },
                            // Automatic fallback uses only the base encoder candidate.
                            workspace: streaming_workspace(settings, false),
                        }
                    },
                )
            }),
            resources,
        )?))
    }

    #[cfg(test)]
    fn members(&self) -> &[MemberPlan] {
        match self {
            Self::IndependentMembers(members) => members,
            _ => panic!("expected independent member plan"),
        }
    }
}

/// The largest member an automatic filter search looks at as a whole.
///
/// Under the shortest-path parse, whole means the member and its filtered copy sit
/// in memory together, which is the only cost left: the search works from samples
/// and the encode spreads across every worker. Any other parse encodes a whole
/// member on one thread, while block execution uses them all, so past a smaller
/// size that stops being worth it. Explicit filters and candidates still run whole,
/// as they ask to.
fn whole_member_search_limit(settings: &CompressPlan) -> u64 {
    let megabytes = if settings.encode_options.optimal_parse {
        256
    } else {
        48
    };
    megabytes * 1024 * 1024
}

fn streaming_workspace(settings: &CompressPlan, include_candidates: bool) -> u64 {
    let optimal = settings.encode_options.optimal_parse
        || (include_candidates
            && settings
                .candidates
                .iter()
                .any(|options| options.optimal_parse));
    super::streaming_lz_workspace(
        settings.dictionary_size,
        crate::codec::rar50::MAX_LZ_BLOCK_SIZE,
        optimal,
    )
}

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct CompressPlan {
    pub(super) algorithm_version: u8,
    pub(super) encode_options: EncodeOptions,
    pub(super) dictionary_size: u64,
    pub(super) block_size: usize,
    pub(super) solid: bool,
    /// The RAR 5 compression method. Method zero means the members are stored
    /// verbatim, so nothing is compressed at all.
    pub(super) method: u8,
    /// Filters and multi-candidate encoding both need the whole member at
    /// once, so they only run for members that fit the memory budget.
    pub(super) filter_policy: FilterPolicy,
    pub(super) candidates: Records<EncodeOptions>,
}

/// Working memory a member needs to be filtered as a whole: the member, the
/// filtered copy, and the candidate packed outputs being compared.
pub(super) fn whole_member_workspace(input_size: u64, plan: &CompressPlan) -> u64 {
    let optimal = plan.encode_options.optimal_parse
        || plan.candidates.iter().any(|options| options.optimal_parse);
    let reach = plan
        .candidates
        .iter()
        .map(|options| options.max_match_distance as u64)
        .chain(std::iter::once(
            plan.encode_options.max_match_distance as u64,
        ))
        .max()
        .unwrap_or(0)
        .min(input_size)
        .max(crate::codec::rar50::LZ_BLOCK_SIZE as u64);
    let block = input_size.min(crate::codec::rar50::MAX_LZ_BLOCK_SIZE as u64) as usize;
    // Input, transformed input and competing packed outputs stay live alongside
    // the finder and the per-block token/parse workspace, not instead of them.
    input_size
        .saturating_mul(4)
        .saturating_add(super::streaming_lz_workspace(reach, block, optimal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> CompressPlan {
        let encode_options = EncodeOptions::new(8).with_max_match_distance(128 * 1024);
        CompressPlan {
            algorithm_version: 0,
            encode_options,
            dictionary_size: 128 * 1024,
            block_size: crate::codec::rar50::LZ_BLOCK_SIZE,
            solid: false,
            method: 1,
            filter_policy: FilterPolicy::Auto,
            candidates: vec![encode_options].into(),
        }
    }

    #[test]
    fn automatic_fallback_records_the_admission_boundary_and_reason() {
        let settings = settings();
        let size = 2 * 1024 * 1024;
        let required = whole_member_workspace(size, &settings);
        let at_limit = ExecutionPlan::new(&settings, [size].into_iter(), required);
        assert_eq!(at_limit.members()[0].execution, Execution::WholeMember);
        let below = ExecutionPlan::new(&settings, [size].into_iter(), required - 1);
        assert_eq!(
            below.members()[0].execution,
            Execution::Blocks {
                fallback: FallbackReason::AutomaticFilterWorkspace {
                    required,
                    limit: required - 1
                },
            }
        );
        assert!(below.members()[0].workspace < required);
    }

    #[test]
    fn explicit_filters_and_candidate_trials_cannot_silently_fall_back() {
        let mut settings = settings();
        settings.filter_policy =
            FilterPolicy::Explicit(crate::FilterSpec::whole(crate::FilterKind::Delta {
                channels: 1,
            }));
        for filter in [settings.filter_policy.clone(), FilterPolicy::None] {
            settings.filter_policy = filter;
            settings
                .candidates
                .push_growing(settings.encode_options)
                .unwrap();
            let execution = ExecutionPlan::new(&settings, [16].into_iter(), 1);
            assert_eq!(execution.members()[0].execution, Execution::WholeMember);
            assert!(execution.members()[0].workspace > 1);
        }
    }

    #[test]
    fn large_members_use_blocks_even_when_the_budget_would_fit_them() {
        let mut automatic = settings();
        automatic.filter_policy = FilterPolicy::Auto;
        let resources = WriterResources::new(u64::MAX / 2);
        let plan = |size| {
            let execution =
                ExecutionPlan::with_resources(&automatic, [size].into_iter(), &resources).unwrap();
            execution.members()[0].execution
        };
        let search_limit = whole_member_search_limit(&automatic);
        assert_eq!(plan(search_limit - 1), Execution::WholeMember);
        assert!(matches!(plan(search_limit), Execution::Blocks { .. }));

        // An explicit filter was asked for by name, so it isn't traded for speed.
        let mut explicit = settings();
        explicit.filter_policy = FilterPolicy::explicit(crate::rar50::FilterKind::E8);
        let execution =
            ExecutionPlan::with_resources(&explicit, [search_limit].into_iter(), &resources)
                .unwrap();
        assert_eq!(execution.members()[0].execution, Execution::WholeMember);
    }

    #[test]
    fn stored_empty_and_solid_modes_do_not_acquire_whole_member_workspace() {
        let mut settings = settings();
        let empty = ExecutionPlan::new(&settings, [0].into_iter(), 0);
        assert_eq!(empty.members()[0].workspace, 0);
        assert_eq!(empty.members()[0].execution, Execution::WholeMember);
        settings.method = 0;
        let stored = ExecutionPlan::new(&settings, [u64::MAX].into_iter(), 0);
        assert!(matches!(stored, ExecutionPlan::Stored));
        settings.method = 1;
        settings.solid = true;
        let solid = ExecutionPlan::new(&settings, [16, u64::MAX].into_iter(), u64::MAX);
        assert!(matches!(solid, ExecutionPlan::Blocks { workspace } if workspace > 0));
    }

    #[test]
    fn independent_member_planning_respects_the_execution_ledger() {
        let ledger = crate::codec::workspace::Allowance::limited(0);
        let resources = WriterResources::default().with_execution_allowance(ledger.clone());
        let error = ExecutionPlan::with_resources(&settings(), [1].into_iter(), &resources)
            .err()
            .unwrap();
        assert_eq!(error.kind(), crate::ErrorKind::ResourceLimit);
        assert_eq!(ledger.used(), 0);
    }
}
