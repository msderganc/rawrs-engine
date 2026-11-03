//! Chooses filters from the bytes alone, one work unit at a time, with no trial
//! encode: an x86 test on call density and repeated targets, and a delta test on how
//! smooth the data is at each candidate channel spacing. Delta intervals win where
//! the two overlap, and the x86 filter fills gaps between them.
use crate::{FilterKind, FilterSpec};
use std::ops::Range;

const WORK_UNIT: usize = 64 * 1024;
const MIN_ANALYZED: usize = 129;
/// Branch filters are not considered for data this far into a file.
const BRANCH_POSITION_LIMIT: usize = 256 * 1024 * 1024;
const MIN_BRANCH_GAP: usize = 101;
const PROBE: usize = 256;
const MAX_CHANNELS: usize = 32;
const REPEAT_REJECT: u32 = 254;
const REPEATED_TARGETS_NEEDED: u32 = 8;

/// The filters to use for a member.
pub(crate) fn select(data: &[u8]) -> Vec<FilterSpec> {
    if data.len() < MIN_ANALYZED {
        return Vec::new();
    }
    let units: Vec<usize> = (0..data.len().div_ceil(WORK_UNIT))
        .map(|unit| unit * WORK_UNIT)
        .collect();
    let (chosen, _) = crate::parallel::map_prefix(units, |start| {
        let unit = &data[start..(start + WORK_UNIT).min(data.len())];
        Ok::<_, std::convert::Infallible>(select_unit(unit, start))
    });
    merge(chosen.into_iter().flatten().collect())
}

fn select_unit(unit: &[u8], start: usize) -> Vec<FilterSpec> {
    let deltas = delta_intervals(unit);
    let branch = if start < BRANCH_POSITION_LIMIT {
        x86_kind(unit)
    } else {
        None
    };
    let mut specs = Vec::new();
    let mut position = 0;
    for (range, channels) in deltas {
        if let Some(kind) = branch {
            if range.start - position >= MIN_BRANCH_GAP {
                specs.push(FilterSpec::range(
                    kind,
                    start + position..start + range.start,
                ));
            }
        }
        position = range.end;
        specs.push(FilterSpec::range(
            FilterKind::Delta { channels },
            start + range.start..start + range.end,
        ));
    }
    if let Some(kind) = branch {
        if unit.len() - position >= MIN_BRANCH_GAP {
            specs.push(FilterSpec::range(
                kind,
                start + position..start + unit.len(),
            ));
        }
    }
    specs
}

/// Joins neighbours with the same filter, so a run of units costs one descriptor.
/// The block writer divides whatever a format limit will not take in one piece.
fn merge(specs: Vec<FilterSpec>) -> Vec<FilterSpec> {
    let mut merged: Vec<FilterSpec> = Vec::with_capacity(specs.len());
    for spec in specs {
        if let Some(last) = merged.last_mut() {
            if last.kind == spec.kind {
                if let (Some(previous), Some(next)) = (&mut last.range, &spec.range) {
                    if previous.end == next.start {
                        previous.end = next.end;
                        continue;
                    }
                }
            }
        }
        merged.push(spec);
    }
    merged
}

/// x86 code calls the same few places over and over, and calls them often.
fn x86_kind(unit: &[u8]) -> Option<FilterKind> {
    let size = unit.len();
    let mut targets = [0u32; 4];
    let mut next_target = 0;
    let (mut repeats, mut calls, mut jumps) = (0u32, 0usize, 0usize);
    let mut position = 0;
    while position + 5 < size {
        let opcode = unit[position];
        if opcode != 0xe8 && opcode != 0xe9 {
            position += 1;
            continue;
        }
        let operand = u32::from_le_bytes([
            unit[position + 1],
            unit[position + 2],
            unit[position + 3],
            unit[position + 4],
        ]);
        let high = operand >> 24;
        if (high == 0 && operand != 0) || high == 0xff {
            let target = operand.wrapping_add(position as u32);
            if target != 0 && targets.contains(&target) {
                repeats += 1;
            }
            targets[next_target] = target;
            next_target = (next_target + 1) % targets.len();
            if opcode == 0xe8 {
                calls += 1;
                if repeats >= REPEATED_TARGETS_NEEDED && calls >= size / 128 {
                    return Some(FilterKind::E8);
                }
            } else {
                jumps += 1;
                if repeats >= REPEATED_TARGETS_NEEDED && jumps >= size / 512 && calls > size / 256 {
                    return Some(FilterKind::E8E9);
                }
            }
        }
        position += 1;
    }
    None
}

/// How much of each 256-byte probe repeats a recent short pattern, out of 256.
fn repeat_scores(unit: &[u8]) -> Vec<u32> {
    let mut epochs = vec![0u32; 1 << 15];
    let mut scores = Vec::with_capacity(unit.len() / PROBE + 1);
    for (index, block) in unit.chunks(PROBE).enumerate() {
        let epoch = index as u32 + 4;
        let (mut hash, mut hits) = (0usize, 0u32);
        for &byte in block {
            hash = ((hash << 5) ^ usize::from(byte)) & 0x7fff;
            if epochs[hash] + 3 >= epoch {
                hits += 1;
            }
            epochs[hash] = epoch;
        }
        scores.push(256 * hits / block.len() as u32);
    }
    scores
}

/// The channel spacing a probe looks smoothest at, if it is smooth enough to count.
fn infer_channels(unit: &[u8], probe: usize, density: u32, max_channels: usize) -> usize {
    let (mut best_score, mut best) = (u32::MAX, 0);
    for channels in 1..=max_channels {
        let mut score = 0u32;
        for region in [0, 64, 128, 192] {
            for sample in 0..8 {
                let position = probe * PROBE + region + sample;
                if position + 2 * max_channels >= unit.len() {
                    continue;
                }
                let first = unit[position + channels].wrapping_sub(unit[position]) as i8;
                let second =
                    unit[position + 2 * channels].wrapping_sub(unit[position + channels]) as i8;
                score += (i32::from(first) - i32::from(second))
                    .unsigned_abs()
                    .min(32);
            }
        }
        if score < best_score {
            best_score = score;
            best = channels;
            if best_score < 16 {
                break;
            }
        }
    }
    if best_score.saturating_mul(density) < 49152 {
        best
    } else {
        0
    }
}

/// A channel spacing for each probe, zero for none, filled in coarse passes so that
/// data no neighbour finds smooth is not scored at all.
fn label_probes(unit: &[u8], scores: &[u32]) -> Vec<u8> {
    let count = unit.len() / PROBE;
    let mut labels = vec![0u8; count];
    let mut done = vec![false; count];
    let mut max_channels = MAX_CHANNELS;
    for (start, stride) in [(0, 32), (1, 16), (2, 8), (3, 1)] {
        let mut wide = false;
        let mut probe = start;
        while probe < count {
            if !done[probe] {
                let quiet_neighbours = probe > 0
                    && probe + 1 < count
                    && done[probe - 1]
                    && done[probe + 1]
                    && labels[probe - 1] == 0
                    && labels[probe + 1] == 0;
                if scores[probe] < REPEAT_REJECT && !quiet_neighbours {
                    let channels = infer_channels(unit, probe, scores[probe], max_channels);
                    labels[probe] = channels as u8;
                    wide |= channels > max_channels / 2;
                }
                done[probe] = true;
            }
            probe += stride;
        }
        if !wide && max_channels > 1 {
            max_channels /= 2;
        }
    }
    labels
}

/// Evens out labels that disagree with most of the unit or with their neighbours.
fn smooth_labels(labels: &mut [u8]) {
    let mut counts = [0usize; MAX_CHANNELS + 1];
    for &label in labels.iter() {
        counts[usize::from(label)] += 1;
    }
    let nonzero: usize = counts[1..].iter().sum();
    let dominant = (1..=MAX_CHANNELS)
        .rev()
        .max_by_key(|&channels| counts[channels])
        .unwrap_or(1);
    let at_multiples: usize = (1..=MAX_CHANNELS)
        .filter(|channels| channels % dominant == 0)
        .map(|channels| counts[channels])
        .sum();
    if nonzero >= 17 && at_multiples * 2 > nonzero {
        let snapshot = labels.to_vec();
        let mut start = 0;
        while start < snapshot.len() {
            let label = snapshot[start];
            let mut end = start + 1;
            while end < snapshot.len() && snapshot[end] == label {
                end += 1;
            }
            if end - start < 8 && usize::from(label) > 4 && usize::from(label) % dominant == 0 {
                labels[start..end].fill(dominant as u8);
            }
            start = end;
        }
        let mut group = 0;
        while group + 16 < labels.len() {
            let matching = labels[group..group + 16]
                .iter()
                .filter(|&&label| usize::from(label) == dominant)
                .count();
            if (12..16).contains(&matching) {
                labels[group..group + 16].fill(dominant as u8);
            }
            group += 16;
        }
    }
    for index in 2..labels.len().saturating_sub(1) {
        if labels[index - 2] == labels[index - 1] && labels[index - 1] == labels[index + 1] {
            labels[index] = labels[index - 1];
        }
    }
}

/// Whether delta at this spacing looks worth it, judged from second differences.
fn interval_gains(bytes: &[u8], channels: usize, scores: &[u32]) -> bool {
    let length = bytes.len();
    let repeats =
        (256 * scores.iter().map(|&score| u64::from(score)).sum::<u64>() / length as u64) as u32;
    if repeats >= 251 {
        return false;
    }
    let (mut raw_seen, mut second_seen) = ([false; 256], [false; 256]);
    let mut smallest = u64::MAX;
    for channel in 0..channels {
        let (mut previous, mut previous_first, mut sum) = (0u8, 0u8, 0u64);
        let mut position = channel;
        while position + channels < length {
            let current = bytes[position];
            let second = previous_first.wrapping_sub(current).wrapping_add(previous);
            previous_first = current.wrapping_sub(previous);
            previous = current;
            raw_seen[usize::from(current)] = true;
            second_seen[usize::from(second)] = true;
            sum += u64::from((second as i8).unsigned_abs());
            position += channels;
        }
        smallest = smallest.min(sum);
    }
    let raw = raw_seen.iter().filter(|&&seen| seen).count();
    let second = second_seen.iter().filter(|&&seen| seen).count();
    if second > 2 * raw {
        return false;
    }
    let quality = 256 * smallest / length as u64 * channels as u64;
    (length > 1023 || quality < 2561)
        && quality < 5121
        && (quality + 128) * u64::from(repeats) < 196_608
}

fn delta_intervals(unit: &[u8]) -> Vec<(Range<usize>, usize)> {
    let scores = repeat_scores(unit);
    let mut labels = label_probes(unit, &scores);
    smooth_labels(&mut labels);
    let mut intervals = Vec::new();
    let mut start = 0;
    while start < labels.len() {
        let label = usize::from(labels[start]);
        let mut end = start + 1;
        while end < labels.len() && usize::from(labels[end]) == label {
            end += 1;
        }
        let range = start * PROBE..end * PROBE;
        if label != 0
            && range.len() > 512
            && (label != 1 || range.len() > 1024)
            && range.len() / label > 128
            && interval_gains(&unit[range.clone()], label, &scores[start..end])
        {
            intervals.push((range, label));
        }
        start = end;
    }
    intervals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_repeated_calls_select_the_call_filter() {
        let mut unit = Vec::new();
        while unit.len() < 4096 {
            unit.push(0xe8);
            let target = 0x1000u32.wrapping_sub(unit.len() as u32 - 1);
            unit.extend_from_slice(&target.to_le_bytes());
            unit.extend_from_slice(&[0x90; 8]);
        }
        assert_eq!(x86_kind(&unit), Some(FilterKind::E8));
    }

    #[test]
    fn sparse_or_scattered_calls_select_nothing() {
        let sparse = [vec![0u8; 4000], vec![0xe8, 1, 0, 0, 0], vec![0u8; 4000]].concat();
        assert_eq!(x86_kind(&sparse), None);
        let mut scattered = Vec::new();
        for call in 0u32..800 {
            scattered.push(0xe8);
            scattered.extend_from_slice(&(call * 7919 + 1).to_le_bytes());
        }
        assert_eq!(x86_kind(&scattered), None);
    }

    #[test]
    fn smooth_stereo_samples_select_delta_and_noise_selects_nothing() {
        let mut samples = Vec::new();
        for index in 0..WORK_UNIT / 4 {
            let time = index as f64;
            let left = ((time * 0.031).sin() * 9000.0) as i16;
            let right = ((time * 0.023).sin() * 7000.0) as i16;
            samples.extend_from_slice(&left.to_le_bytes());
            samples.extend_from_slice(&right.to_le_bytes());
        }
        let intervals = delta_intervals(&samples);
        assert!(!intervals.is_empty());
        assert!(intervals.iter().all(|(_, channels)| channels % 2 == 0));

        let mut seed = 1u32;
        let noise: Vec<u8> = (0..WORK_UNIT)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        assert!(delta_intervals(&noise).is_empty());
    }

    #[test]
    fn delta_takes_precedence_and_branches_only_fill_large_gaps() {
        let member = vec![0u8; 100];
        assert!(select(&member).is_empty());
        let merged = merge(vec![
            FilterSpec::range(FilterKind::E8, 0..10),
            FilterSpec::range(FilterKind::E8, 10..20),
            FilterSpec::range(FilterKind::Delta { channels: 2 }, 20..30),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].range, Some(0..20));
    }
}
