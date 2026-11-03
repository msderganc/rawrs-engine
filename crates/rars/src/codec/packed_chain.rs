//! A bounded job's exact hash chains, stored as reverse-readable delta streams.
//! Construction sees the captured input twice. Insertion only publishes the
//! already-built prefix of one bucket; future positions remain unreachable.

use super::workspace::{Allowance, Budget, Buffer};
use super::{Error, Result};

const HASH_BITS: u32 = 17;
const BUCKETS: usize = 1 << HASH_BITS;
const NO_LINK: u32 = u32::MAX;

#[inline]
fn hash(input: &[u8], pos: usize) -> usize {
    let value = u32::from_le_bytes(input[pos..pos + 4].try_into().unwrap());
    (value.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

#[inline]
fn encoded_length(delta: u32) -> usize {
    match delta {
        0..=32_767 => 2,
        32_768..=1_073_741_823 => 4,
        _ => 6,
    }
}

fn encode(output: &mut [u8], delta: u32) {
    let last = output.len() / 2 - 1;
    for (index, bytes) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let mut unit = ((delta >> (index * 15)) & 0x7FFF) as u16;
        if index != last {
            unit |= 0x8000;
        }
        bytes.copy_from_slice(&unit.to_le_bytes());
    }
}

#[inline]
fn decode_before(input: &[u8], end: usize) -> (u32, usize) {
    debug_assert_eq!(end & 1, 0);
    debug_assert_eq!(
        u16::from_le_bytes(input[end - 2..end].try_into().unwrap()) & 0x8000,
        0
    );
    if end < 4 {
        return decode_before_scalar(input, end);
    }
    let word = u32::from_le_bytes(input[end - 4..end].try_into().unwrap());
    let length = if word & 0x8000 == 0 { 2 } else { 4 };
    if length == 4
        && end >= 6
        && u16::from_le_bytes(input[end - 6..end - 4].try_into().unwrap()) & 0x8000 != 0
    {
        return decode_before_scalar(input, end);
    }
    let shifted = word >> ((4 - length) * 8);
    ((shifted & 0x7FFF) | ((shifted >> 1) & 0x3FFF_8000), length)
}

#[inline]
fn decode_before_scalar(input: &[u8], end: usize) -> (u32, usize) {
    debug_assert_eq!(end & 1, 0);
    let mut start = end - 2;
    let mut delta = u32::from(u16::from_le_bytes(input[start..end].try_into().unwrap()));
    debug_assert_eq!(delta & 0x8000, 0);
    while start >= 2 {
        let previous = u16::from_le_bytes(input[start - 2..start].try_into().unwrap());
        if previous & 0x8000 == 0 {
            break;
        }
        start -= 2;
        delta = (delta << 15) | u32::from(previous & 0x7FFF);
    }
    (delta, end - start)
}

#[derive(Debug)]
pub(crate) struct PackedMatchFinder<B: Budget = Allowance> {
    cursor: Buffer<u32, B>,
    packed: Buffer<u8, B>,
    next_position: usize,
}

impl<B: Budget> PackedMatchFinder<B> {
    pub(crate) fn with_allowance(input: &[u8], allowance: &B) -> Result<Self> {
        u32::try_from(input.len())
            .map_err(|_| Error::InvalidData("packed finder input exceeds u32 positions"))?;
        let mut head = Buffer::filled(BUCKETS, NO_LINK, allowance)?;
        let mut cursor = Buffer::filled(BUCKETS, 0u32, allowance)?;
        let positions = input.len().saturating_sub(3);
        for pos in 0..positions {
            let bucket = hash(input, pos);
            let previous = head[bucket];
            let delta = if previous == NO_LINK {
                0
            } else {
                pos as u32 - previous
            };
            cursor[bucket] = cursor[bucket]
                .checked_add(encoded_length(delta) as u32)
                .ok_or(Error::InvalidData(
                    "packed finder bucket exceeds u32 offsets",
                ))?;
            head[bucket] = pos as u32;
        }
        let mut total = 0u32;
        for count in cursor.iter_mut() {
            let length = *count;
            *count = total;
            total = total.checked_add(length).ok_or(Error::InvalidData(
                "packed finder storage exceeds u32 offsets",
            ))?;
        }
        let mut packed = Buffer::filled(total as usize, 0u8, allowance)?;
        head.fill(NO_LINK);
        for pos in 0..positions {
            let bucket = hash(input, pos);
            let previous = head[bucket];
            let delta = if previous == NO_LINK {
                0
            } else {
                pos as u32 - previous
            };
            let length = encoded_length(delta);
            let start = cursor[bucket] as usize;
            encode(&mut packed[start..start + length], delta);
            cursor[bucket] += length as u32;
            head[bucket] = pos as u32;
        }
        // Each final bucket end is the next bucket's initial start, including
        // empty buckets. Recover those starts without another input scan.
        for bucket in (1..BUCKETS).rev() {
            cursor[bucket] = cursor[bucket - 1];
        }
        cursor[0] = 0;
        // Head positions were needed only to construct exact deltas. Runtime
        // insertion finds the next record boundary directly in its stream.
        drop(head);
        Ok(Self {
            cursor,
            packed,
            next_position: 0,
        })
    }

    pub(crate) fn allowance(&self) -> B {
        self.packed.allowance()
    }

    /// Only dense, increasing positions from this same captured input are
    /// supported. The returned cursor ends at this position's own delta.
    #[inline]
    pub(crate) fn insert(&mut self, input: &[u8], pos: usize) -> usize {
        debug_assert_eq!(pos, self.next_position);
        debug_assert!(pos + 4 <= input.len());
        let bucket = hash(input, pos);
        let mut end = self.cursor[bucket] as usize;

        loop {
            let unit = u16::from_le_bytes(self.packed[end..end + 2].try_into().unwrap());
            end += 2;
            if unit & 0x8000 == 0 {
                break;
            }
        }
        self.cursor[bucket] = end as u32;
        self.next_position = pos + 1;
        self.cursor[bucket] as usize
    }

    /// Replay the ordinary chain's nearest-first budget, including collisions
    /// and sub-four-byte probe state. Store only parser-usable runs. No
    /// allocation occurs except when the caller's output needs capacity.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn collect_improving(
        &self,
        input: &[u8],
        pos: usize,
        end_cursor: usize,
        max_length: usize,
        max_distance: usize,
        cut: usize,
        nice: usize,
        out: &mut Buffer<(u32, u32), B>,
    ) -> std::result::Result<(), B::Failure> {
        debug_assert!(max_length >= 4 && pos + max_length <= input.len());
        debug_assert_eq!(self.next_position, pos + 1);
        let prefix = u32::from_le_bytes(input[pos..pos + 4].try_into().unwrap());
        let mut candidate = pos;
        let mut cursor = end_cursor;
        let mut longest = 0;
        let mut checked = 0;
        while checked < cut && longest < max_length && longest < nice {
            let (delta, length) = decode_before(&self.packed, cursor);
            cursor -= length;
            if delta == 0 {
                break;
            }
            candidate -= delta as usize;
            let distance = pos - candidate;
            if distance > max_distance {
                break;
            }
            checked += 1;
            // Any improving match shares the byte at the previous best
            // length, including sub-four-byte collision state.
            if input[candidate + longest] != input[pos + longest] {
                continue;
            }
            let difference =
                prefix ^ u32::from_le_bytes(input[candidate..candidate + 4].try_into().unwrap());
            let length = if difference != 0 {
                (difference.trailing_zeros() / 8) as usize
            } else {
                4 + super::fast::match_length(input, pos + 4, distance, max_length - 4)
            };
            if length > longest {
                if length >= 4 {
                    out.push((length as u32, distance as u32))?;
                }
                longest = length;
            }
        }
        Ok(())
    }

    /// Keep the complete packed-chain walk outside parser/pricing frames. The
    /// caller preallocates one start per block position plus the terminal start.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn collect_block(
        &mut self,
        input: &[u8],
        block: std::ops::Range<usize>,
        distance_limit: usize,
        cut: usize,
        nice: usize,
        max_match_length: usize,
        runs: &mut Buffer<(u32, u32), B>,
        starts: &mut Buffer<u32, B>,
    ) -> std::result::Result<(), B::Failure> {
        debug_assert!(block.start <= block.end && block.end <= input.len());
        let mut committed_through = block.start;
        for pos in block.clone() {
            starts.push_admitted(runs.len() as u32);
            // Dense insertion includes NICE interiors and nonfinal block tails
            // that are too short to search but still have a full input prefix.
            if input.len() - pos < 4 {
                continue;
            }
            let cursor = self.insert(input, pos);
            let remaining = block.end - pos;
            let max_length = remaining.min(max_match_length);
            let max_distance = pos.min(distance_limit);
            let before = runs.len();
            if pos >= committed_through && cut != 0 && max_distance != 0 && max_length >= 4 {
                self.collect_improving(
                    input,
                    pos,
                    cursor,
                    max_length,
                    max_distance,
                    cut,
                    nice,
                    runs,
                )?;
            }
            if let Some(&(length, _)) = runs[before..].last() {
                let reach = (length as usize).min(remaining).min(max_match_length);
                if reach >= nice {
                    committed_through = pos + reach;
                }
            }
        }
        starts.push_admitted(runs.len() as u32);
        if block.end == input.len() {
            // This captured index has no future readable position. Retire it
            // before the caller allocates the final block's pricing workspace.
            let allowance = self.packed.allowance();
            self.packed = Buffer::new(&allowance);
            self.cursor = Buffer::new(&allowance);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::match_finder::{MatchFinder, NO_POSITION};
    use crate::codec::workspace::RefusingBudget;

    #[test]
    fn reverse_delta_codes_cover_all_width_boundaries() {
        let values = [0, 1, 32_767, 32_768, 1_073_741_823, 1_073_741_824, u32::MAX];
        let mut bytes = Vec::new();
        for &value in &values {
            let start = bytes.len();
            bytes.resize(start + encoded_length(value), 0);
            encode(&mut bytes[start..], value);
        }
        let mut end = bytes.len();
        for &value in values.iter().rev() {
            let (actual, length) = decode_before(&bytes, end);
            assert_eq!(actual, value);
            assert_eq!(length, encoded_length(value));
            end -= length;
        }
        assert_eq!(end, 0);
    }

    #[test]
    fn broadword_decode_matches_scalar_on_mixed_records_and_bucket_sentinels() {
        let mut bytes = Vec::new();
        for value in 0..=u16::MAX as u32 {
            if value % 257 == 0 {
                // A new bucket starts with a zero sentinel, including at
                // global offset zero; empty buckets add no physical bytes.
                let start = bytes.len();
                let length = encoded_length(0);
                bytes.resize(start + length, 0);
                encode(&mut bytes[start..], 0);
                assert_eq!(decode_before(&bytes, bytes.len()), (0, length));
            }
            for delta in [value, value << 14, u32::MAX - value] {
                let start = bytes.len();
                let length = encoded_length(delta);
                bytes.resize(start + length, 0);
                encode(&mut bytes[start..], delta);
                let end = bytes.len();
                assert_eq!(decode_before(&bytes, end), (delta, length));
                assert_eq!(
                    decode_before(&bytes, end),
                    decode_before_scalar(&bytes, end)
                );
            }
        }
    }

    #[test]
    fn wide_leb_canonical_boundaries_and_reverse_records() {
        let cases: &[(u32, &[u8])] = &[
            (0, &[0, 0]),
            (32_767, &[0xFF, 0x7F]),
            (32_768, &[0, 0x80, 1, 0]),
            (1_073_741_823, &[0xFF, 0xFF, 0xFF, 0x7F]),
            (1_073_741_824, &[0, 0x80, 0, 0x80, 1, 0]),
            (u32::MAX, &[0xFF, 0xFF, 0xFF, 0xFF, 3, 0]),
        ];
        let mut bytes = Vec::new();
        for &(delta, expected) in cases {
            let start = bytes.len();
            bytes.resize(start + encoded_length(delta), 0);
            encode(&mut bytes[start..], delta);
            assert_eq!(&bytes[start..], expected);
            assert_eq!(decode_before(&bytes, bytes.len()), (delta, expected.len()));
            assert_eq!(
                decode_before_scalar(&bytes, bytes.len()),
                (delta, expected.len())
            );
        }
        let mut end = bytes.len();
        for &(delta, expected) in cases.iter().rev() {
            assert_eq!(decode_before(&bytes, end), (delta, expected.len()));
            end -= expected.len();
        }
        assert_eq!(end, 0);
    }

    #[test]
    fn packed_empty_buckets_and_first_global_record_have_safe_boundaries() {
        let allowance = Allowance::default();
        let empty = PackedMatchFinder::with_allowance(b"abc", &allowance).unwrap();
        assert!(empty.packed.is_empty());
        assert!(empty.cursor.iter().all(|&cursor| cursor == 0));
        let input = b"aaaazzzzaaaa";
        let mut finder = PackedMatchFinder::with_allowance(input, &allowance).unwrap();
        assert!(finder.cursor.windows(2).any(|pair| pair[0] == pair[1]));
        let sentinel_length = encoded_length(0);
        assert_eq!(
            decode_before(&finder.packed, sentinel_length),
            (0, sentinel_length)
        );
        for pos in 0..input.len() - 3 {
            let end = finder.insert(input, pos);
            let (delta, _) = decode_before(&finder.packed, end);
            assert!(delta == 0 || delta as usize <= pos);
        }
    }

    fn fixture(length: usize) -> Vec<u8> {
        let mut state = 0x7A59_1234_AA55_0123u64;
        let mut input = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            input.push(state as u8);
        }
        for pos in (0..length.saturating_sub(32)).step_by(97) {
            input[pos..pos + 16].copy_from_slice(b"repeated-prefix!");
        }
        input
    }

    fn compare(input: &[u8], seed: usize, window: usize) {
        let allowance = Allowance::default();
        let mut packed = PackedMatchFinder::with_allowance(input, &allowance).unwrap();
        let mut wide = MatchFinder::<4>::new(window);
        assert_eq!(packed.next_position, 0);
        for pos in 0..seed {
            packed.insert(input, pos);
            wide.insert(input, pos);
        }
        for pos in seed..input.len() - 3 {
            let end = packed.insert(input, pos);
            wide.insert(input, pos);
            if pos % 7 != 0 {
                continue;
            }
            for cut in [0, 1, 8, 128] {
                for (cap, nice) in [(4, 512), (32, 16), (512, 512)] {
                    let max_length = cap.min(input.len() - pos);
                    if max_length < 4 {
                        continue;
                    }
                    let max_distance = pos.min(window);
                    let mut expected = Buffer::new(&allowance);
                    let mut actual = Buffer::new(&allowance);
                    let mut longest = 0;
                    let mut checked = 0;
                    let mut candidate = wide.first(input, pos);
                    while candidate != NO_POSITION && longest < max_length && longest < nice {
                        if candidate >= pos {
                            candidate = wide.previous(candidate);
                            continue;
                        }
                        let distance = pos - candidate;
                        if distance > max_distance || checked == cut {
                            break;
                        }
                        checked += 1;
                        if input[candidate + longest] == input[pos + longest] {
                            let length =
                                super::super::fast::match_length(input, pos, distance, max_length);
                            if length > longest {
                                if length >= 4 {
                                    expected.push((length as u32, distance as u32)).unwrap();
                                }
                                longest = length;
                            }
                        }
                        if checked == cut {
                            break;
                        }
                        candidate = wide.previous(candidate);
                    }
                    packed
                        .collect_improving(
                            input,
                            pos,
                            end,
                            max_length,
                            max_distance,
                            cut,
                            nice,
                            &mut actual,
                        )
                        .unwrap();
                    assert_eq!(
                        &actual[..],
                        &expected[..],
                        "pos{pos} cut{cut} cap{cap} window{window}"
                    );
                }
            }
        }
    }

    #[test]
    fn packed_frontiers_match_original_hash_chains() {
        let random = fixture(8192);
        let pattern: Vec<_> = (0..8192)
            .map(|pos| b"abcdabcdXabcdabcdY"[pos % 18])
            .collect();
        for input in [&random[..], &pattern[..]] {
            for window in [32, 4096, 16384] {
                compare(input, 0, window);
            }
        }
    }

    #[test]
    fn packed_collision_probe_keeps_subfour_state_cut_and_refusal_order() {
        let target = hash(b"abcd", 0);
        let zero = (0u32..1 << 24)
            .find_map(|value| {
                let word = value.to_le_bytes();
                (word[0] != b'a' && hash(&word, 0) == target).then_some(word)
            })
            .unwrap();
        let one = (0u32..1 << 24)
            .find_map(|upper| {
                let word = (u32::from(b'a') | (upper << 8)).to_le_bytes();
                (word[1] != b'b' && hash(&word, 0) == target).then_some(word)
            })
            .unwrap();
        let mut input = vec![0; 75];
        input[..12].copy_from_slice(b"abcdlongtail");
        input[16..21].copy_from_slice(b"abcdX");
        input[32..36].copy_from_slice(&one);
        input[48..52].copy_from_slice(&zero);
        input[63..].copy_from_slice(b"abcdlongtail");
        // Position 63 is included in this exhaustive-budget/cap oracle. Its
        // nearest collisions first prove zero and then one matching byte.
        compare(&input, 0, 128);

        let budget = RefusingBudget::new(usize::MAX);
        let mut finder = PackedMatchFinder::with_allowance(&input, &budget).unwrap();
        let constructor_attempts = budget.attempts();
        let mut end = 0;
        for pos in 0..=63 {
            end = finder.insert(&input, pos);
        }
        let mut runs = Buffer::new(&budget);
        finder
            .collect_improving(&input, 63, end, 12, 63, 4, 512, &mut runs)
            .unwrap();
        assert_eq!(&runs[..], &[(4, 47), (12, 63)]);
        runs.clear();
        finder
            .collect_improving(&input, 63, end, 12, 63, 2, 512, &mut runs)
            .unwrap();
        assert!(runs.is_empty(), "both collisions consume the visit budget");
        drop((runs, finder));
        assert_eq!(budget.used(), 0);

        // One admitted output slot lets the first legal run survive; growing
        // for the second legal run refuses at exactly the same frontier point.
        let budget = RefusingBudget::new(constructor_attempts + 1);
        let mut finder = PackedMatchFinder::with_allowance(&input, &budget).unwrap();
        let mut runs = Buffer::with_capacity(1, &budget).unwrap();
        for pos in 0..=63 {
            end = finder.insert(&input, pos);
        }
        assert_eq!(
            finder.collect_improving(&input, 63, end, 12, 63, 4, 512, &mut runs),
            Err(super::super::Error::Cancelled)
        );
        assert_eq!(&runs[..], &[(4, 47)]);
        drop((runs, finder));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn packed_seeded_history_preserves_expiry_and_future_visibility() {
        let mut input = fixture(8192);
        input.copy_within(100..900, 4096);
        compare(&input, 4096, 256);
        compare(&input, 4096, 8192);
    }

    #[test]
    fn packed_allocations_are_exact_charged_and_refusal_releases_them() {
        let input = fixture(2048);
        for fail_at in 0..4 {
            let allowance = RefusingBudget::new(fail_at);
            let finder = PackedMatchFinder::with_allowance(&input, &allowance);
            drop(finder);
            assert_eq!(allowance.used(), 0, "refusal{fail_at}");
        }
        let allowance = Allowance::limited(u64::MAX);
        let mut finder = PackedMatchFinder::with_allowance(&input, &allowance).unwrap();
        assert_eq!(finder.packed.capacity(), finder.packed.len());
        let charged = allowance.used();
        assert_eq!(charged, (BUCKETS * 4 + finder.packed.len()) as u64);
        for pos in 0..input.len() - 3 {
            finder.insert(&input, pos);
        }
        assert_eq!(allowance.used(), charged, "insertion cannot allocate");
        drop(finder);
        assert_eq!(allowance.used(), 0);
    }

    #[test]
    fn packed_terminal_block_releases_index_but_keeps_match_output() {
        let input = vec![b'x'; 80];
        let allowance = Allowance::limited(u64::MAX);
        let mut finder = PackedMatchFinder::with_allowance(&input, &allowance).unwrap();
        for pos in 0..4 {
            finder.insert(&input, pos);
        }
        let index_bytes = allowance.used();
        let mut runs = Buffer::with_capacity(80, &allowance).unwrap();
        let mut starts = Buffer::with_capacity(81, &allowance).unwrap();
        let output_bytes = allowance.used() - index_bytes;
        finder
            .collect_block(&input, 4..20, 64, 128, 512, 16, &mut runs, &mut starts)
            .unwrap();
        assert_eq!(allowance.used(), index_bytes + output_bytes);
        assert!(!finder.packed.is_empty() && !finder.cursor.is_empty());
        assert_eq!(finder.next_position, 20);
        finder
            .collect_block(&input, 20..80, 64, 128, 512, 16, &mut runs, &mut starts)
            .unwrap();
        assert!(finder.packed.is_empty() && finder.cursor.is_empty());
        assert_eq!(allowance.used(), output_bytes);
        assert_eq!(finder.next_position, input.len() - 3);

        // Equal bytes always yield the nearest distance1 and the exact cap.
        // Check both accumulated blocks after their index has been released.
        let mut expected_runs = Vec::new();
        let mut expected_starts = Vec::new();
        for block in [4..20, 20..80] {
            for pos in block.clone() {
                expected_starts.push(expected_runs.len() as u32);
                let length = (block.end - pos).min(16);
                if length >= 4 {
                    expected_runs.push((length as u32, 1));
                }
            }
            expected_starts.push(expected_runs.len() as u32);
        }
        assert_eq!(&runs[..], &expected_runs);
        assert_eq!(&starts[..], &expected_starts);
        drop(runs);
        drop(starts);
        assert_eq!(allowance.used(), 0);
    }
}
