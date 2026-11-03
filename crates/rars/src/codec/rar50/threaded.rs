//! The two-stage decoder for large members.
//!
//! Workers turn blocks into records with their own Huffman tables and bit readers.
//! They never touch the history. One ordered stage then resolves the remembered
//! distances, copies the matches, schedules the filters and writes the output, so
//! everything one block owes another is still settled in stream order.
//!
//! A worker's failure is only a record of where its block went bad. Blocks are read
//! and decoded ahead of the output, and the sequential decoder would never have seen
//! anything past the member's declared size, so nothing read ahead can fail the
//! member until the ordered stage actually reaches it.
use super::*;

/// The smallest member worth starting workers for.
const THREADED_MIN_OUTPUT: usize = if cfg!(test) {
    1024 * 1024
} else {
    16 * 1024 * 1024
};
/// The smallest member worth starting workers for. A solid member has no other
/// parallelism to give up, since members must decode in order.
pub(super) fn min_output(solid: bool) -> usize {
    if solid {
        SOLID_THREADED_MIN_OUTPUT
    } else {
        THREADED_MIN_OUTPUT
    }
}

const SOLID_THREADED_MIN_OUTPUT: usize = 256 * 1024;

/// The largest block decoded into records. One bit can be one record, so a larger
/// block goes through the sequential decoder rather than into staging memory.
const THREADED_MAX_BLOCK: usize = 128 * 1024;

enum BlockRecord {
    Literals(u32),
    Filter(u32),
    LastMatch,
    Recent {
        index: u8,
        length: u32,
    },
    /// In parts to keep a record to 12 bytes, as blocks stage thousands of them.
    New {
        distance_high: u16,
        length: u32,
        distance_low: u32,
    },
}

/// The largest distance a record holds. No dictionary comes near it, so a larger
/// one is out of the window either way and gets the same treatment at this size.
const MAX_RECORD_DISTANCE: u64 = (1 << 48) - 1;

/// Blocks' buffers on their way back to the workers, so that staging reuses memory
/// instead of releasing it and faulting it back in for every block.
type SpareBlocks = std::sync::Mutex<Vec<DecodedBlock>>;

#[derive(Default)]
struct DecodedBlock {
    records: Vec<BlockRecord>,
    literals: Vec<u8>,
    /// Starts are relative to where the ordered stage is when it reaches them.
    filters: Vec<PendingFilter>,
    failure: Option<Error>,
    is_last: bool,
}

/// A block that brought tables, and the blocks after it that reuse them.
struct BlockGroup<B: Budget> {
    tables: Option<ReaderTables<B>>,
    blocks: Vec<OwnedCompressedBlock<B>>,
}

struct DecodedGroup<B: Budget> {
    tables: Option<ReaderTables<B>>,
    blocks: Vec<DecodedBlock>,
}

/// Where the ordered stage puts what it resolves.
pub(super) trait RecordOutput {
    type Error: From<Error>;
    fn written(&self) -> usize;
    fn literals(&mut self, bytes: &[u8]) -> std::result::Result<(), Self::Error>;
    fn copy_match(
        &mut self,
        distance: usize,
        length: usize,
    ) -> std::result::Result<(), Self::Error>;
    fn filter(&mut self, filter: PendingFilter) -> std::result::Result<(), Self::Error>;
}

/// The stream state the two stages share with the sequential decoder.
pub(super) struct StreamState<'a, B: Budget> {
    pub(super) tables: &'a mut Option<ReaderTables<B>>,
    pub(super) reps: &'a mut [usize; 4],
    pub(super) last_length: &'a mut usize,
    pub(super) control: &'a crate::read_control::ReadControl,
}

pub(super) enum ThreadedEnd<B: Budget> {
    /// The output is complete or the stream ended.
    Finished,
    /// This block is too large to stage. The sequential decoder takes it next.
    Oversized(OwnedCompressedBlock<B>),
}

fn decode_records<B: Budget>(
    block: &OwnedCompressedBlock<B>,
    start_bit: usize,
    tables: &ReaderTables<B>,
    out: &mut DecodedBlock,
) -> Result<()> {
    let mut bits = BitReader::new(&block.payload);
    bits.bit_pos = start_bit;
    let mut run = 0u32;
    let mut decode = || -> Result<()> {
        while bits.bit_pos < block.header.payload_bits {
            let symbol = tables.main.decode(&mut bits)?;
            if symbol < 256 {
                out.literals.push(symbol as u8);
                run += 1;
                continue;
            }
            if run != 0 {
                out.records.push(BlockRecord::Literals(run));
                run = 0;
            }
            match symbol {
                256 => {
                    let index = out.filters.len() as u32;
                    out.filters.push(read_filter(&mut bits, 0)?);
                    out.records.push(BlockRecord::Filter(index));
                }
                257 => out.records.push(BlockRecord::LastMatch),
                258..=261 => {
                    let length_slot = tables.length.decode(&mut bits)?;
                    let length_extra = bits.read_bits(length_slot_extra_bits(length_slot))?;
                    out.records.push(BlockRecord::Recent {
                        index: (symbol - 258) as u8,
                        length: length_from_slot_parts(length_slot, length_extra) as u32,
                    });
                }
                _ => {
                    let length_slot = symbol - 262;
                    let length_extra = bits.read_bits(length_slot_extra_bits(length_slot))?;
                    let length = length_from_slot_parts(length_slot, length_extra);
                    let distance_slot = tables.distance.decode(&mut bits)?;
                    let distance_bit_count = distance_slot_bit_count(distance_slot)?;
                    let distance_extra = if distance_bit_count >= 4 && tables.align_mode {
                        let high = bits.read_bits((distance_bit_count - 4) as u8)?;
                        let low = tables.align.decode(&mut bits)? as u32;
                        (high << 4) | low
                    } else {
                        bits.read_bits(distance_bit_count as u8)?
                    };
                    let distance =
                        distance_from_slot_parts(distance_slot, distance_bit_count, distance_extra);
                    let stored = (distance as u64).min(MAX_RECORD_DISTANCE);
                    out.records.push(BlockRecord::New {
                        distance_high: (stored >> 32) as u16,
                        length: (length + length_bonus(distance)) as u32,
                        distance_low: stored as u32,
                    });
                }
            }
        }
        Ok(())
    };
    let outcome = decode();
    if run != 0 {
        out.records.push(BlockRecord::Literals(run));
    }
    outcome
}

/// Decodes one group. It stops at the first failure, which the block records.
fn decode_group<B: Budget>(
    group: BlockGroup<B>,
    algorithm_version: u8,
    control: &crate::read_control::ReadControl,
    spares: &SpareBlocks,
) -> DecodedGroup<B> {
    let BlockGroup { mut tables, blocks } = group;
    let mut decoded = Vec::with_capacity(blocks.len());
    // Each packed block is dropped once decoded, ahead of the ordered stage.
    for block in blocks {
        let block = &block;
        let mut out = spares
            .lock()
            .expect("taking a spare does not panic")
            .pop()
            .unwrap_or_default();
        out.is_last = block.header.is_last;
        let outcome = (|| -> Result<()> {
            control.check_codec()?;
            let mut start_bit = 0;
            if block.header.has_tables {
                let allowance = block.payload.allowance();
                let (lengths, table_bits) = read_table_lengths_with_allowance(
                    &block.payload,
                    algorithm_version,
                    &allowance,
                )?;
                tables = Some(ReaderTables::from_lengths(&lengths, &allowance)?);
                start_bit = table_bits;
            }
            let reader = tables
                .as_ref()
                .ok_or(Error::InvalidData("RAR 5 block reuses missing tables"))?;
            out.records.reserve(block.payload.len() / 4);
            out.literals.reserve(block.payload.len());
            decode_records(block, start_bit, reader, &mut out)
        })();
        let failed = outcome.is_err();
        out.failure = outcome.err();
        decoded.push(out);
        if failed {
            break;
        }
    }
    DecodedGroup {
        tables,
        blocks: decoded,
    }
}

/// Resolves one block's records into the output, stopping at the declared size.
fn apply_block<B: Budget, O: RecordOutput>(
    block: &DecodedBlock,
    state: &mut StreamState<'_, B>,
    poller: &mut crate::read_control::Poller,
    output: &mut O,
    output_size: usize,
) -> std::result::Result<(), O::Error> {
    let mut next_literal = 0;
    // A staged block is small, so asking once per block is soon enough.
    poller
        .check_codec(output.written())
        .map_err(O::Error::from)?;
    for record in &block.records {
        let written = output.written();
        if written >= output_size {
            return Ok(());
        }
        match *record {
            BlockRecord::Literals(count) => {
                let count = count as usize;
                let take = count.min(output_size - written);
                output.literals(&block.literals[next_literal..next_literal + take])?;
                next_literal += count;
            }
            BlockRecord::Filter(index) => output.filter(block.filters[index as usize])?,
            BlockRecord::LastMatch => {
                if *state.last_length != 0 {
                    output.copy_match(state.reps[0], *state.last_length)?;
                }
            }
            BlockRecord::Recent { index, length } => {
                let index = usize::from(index);
                let distance = state.reps[index];
                if distance == 0 {
                    return Err(O::Error::from(Error::InvalidData(
                        "RAR 5 repeat distance is not initialized",
                    )));
                }
                state.reps[..=index].rotate_right(1);
                state.reps[0] = distance;
                *state.last_length = length as usize;
                output.copy_match(distance, length as usize)?;
            }
            BlockRecord::New {
                distance_high,
                length,
                distance_low,
            } => {
                let distance = (u64::from(distance_high) << 32 | u64::from(distance_low)) as usize;
                state.reps.rotate_right(1);
                state.reps[0] = distance;
                *state.last_length = length as usize;
                output.copy_match(distance, length as usize)?;
            }
        }
    }
    Ok(())
}

/// What stopped a batch before it filled.
enum BatchEnd<B: Budget> {
    ReadFailed(Error),
    Oversized(OwnedCompressedBlock<B>),
}

struct BlockBatch<B: Budget> {
    groups: Vec<BlockGroup<B>>,
    end: Option<BatchEnd<B>>,
    /// Whether nothing follows this batch: it holds the last block, or it ended.
    finished: bool,
}

struct DecodedBatch<B: Budget> {
    groups: Vec<DecodedGroup<B>>,
    end: Option<BatchEnd<B>>,
}

/// Reads up to `count` blocks. `carried` supplies the tables for a first block that
/// does not bring its own.
fn read_batch<B: Budget>(
    input: &mut impl Read,
    allowance: &B,
    count: usize,
    carried: impl FnOnce() -> Option<ReaderTables<B>>,
) -> BlockBatch<B> {
    let mut batch = BlockBatch {
        groups: Vec::new(),
        end: None,
        finished: false,
    };
    let mut carried = Some(carried);
    let mut read = 0;
    while read < count {
        let block = match read_compressed_block_with_allowance(input, allowance) {
            Ok(block) => block,
            Err(error) => {
                batch.end = Some(BatchEnd::ReadFailed(error));
                batch.finished = true;
                break;
            }
        };
        if block.header.payload_size > THREADED_MAX_BLOCK {
            batch.end = Some(BatchEnd::Oversized(block));
            batch.finished = true;
            break;
        }
        let is_last = block.header.is_last;
        if block.header.has_tables || batch.groups.is_empty() {
            let first = batch.groups.is_empty();
            batch.groups.push(BlockGroup {
                tables: if first && !block.header.has_tables {
                    carried.take().and_then(|carried| carried())
                } else {
                    None
                },
                blocks: Vec::with_capacity(1),
            });
        }
        batch
            .groups
            .last_mut()
            .expect("a group was just pushed")
            .blocks
            .push(block);
        read += 1;
        if is_last {
            batch.finished = true;
            break;
        }
    }
    batch
}

fn decode_batch<B: Budget>(
    batch: BlockBatch<B>,
    algorithm_version: u8,
    control: &crate::read_control::ReadControl,
    spares: &SpareBlocks,
) -> DecodedBatch<B> {
    let (groups, _) = crate::parallel::map_prefix(batch.groups, |group| {
        Ok::<_, std::convert::Infallible>(decode_group(group, algorithm_version, control, spares))
    });
    DecodedBatch {
        groups,
        end: batch.end,
    }
}

/// Resolves one decoded batch, returning how the member ended if it did.
fn apply_batch<B: Budget, O: RecordOutput>(
    batch: DecodedBatch<B>,
    state: &mut StreamState<'_, B>,
    poller: &mut crate::read_control::Poller,
    output: &mut O,
    output_size: usize,
    spares: &SpareBlocks,
) -> std::result::Result<Option<ThreadedEnd<B>>, O::Error> {
    for group in batch.groups {
        if group.tables.is_some() {
            *state.tables = group.tables;
        }
        for block in &group.blocks {
            apply_block(block, state, poller, output, output_size)?;
            if output.written() >= output_size {
                return Ok(Some(ThreadedEnd::Finished));
            }
            if let Some(failure) = &block.failure {
                return Err(O::Error::from(failure.clone()));
            }
            if block.is_last {
                return Ok(Some(ThreadedEnd::Finished));
            }
        }
        let mut waiting = spares.lock().expect("returning a spare does not panic");
        for mut block in group.blocks {
            block.records.clear();
            block.literals.clear();
            block.filters.clear();
            waiting.push(block);
        }
    }
    match batch.end {
        Some(BatchEnd::ReadFailed(failure)) => Err(O::Error::from(failure)),
        Some(BatchEnd::Oversized(block)) => Ok(Some(ThreadedEnd::Oversized(block))),
        None => Ok(None),
    }
}

/// Runs batches of blocks through both stages until the member ends or a block is
/// too large to stage. Each batch decodes while the one before it is being applied,
/// so the workers and the ordered stage overlap rather than taking turns.
pub(super) fn decode_blocks<B: Budget, O: RecordOutput>(
    input: &mut impl Read,
    algorithm_version: u8,
    allowance: &B,
    state: &mut StreamState<'_, B>,
    output: &mut O,
    output_size: usize,
) -> std::result::Result<ThreadedEnd<B>, O::Error> {
    // Each staged block costs memory, so batches start small. Where the ordered stage
    // is what takes the time, as it usually is, the workers keep ahead of it anyway.
    // Where they fall behind, as on blocks of nothing but literals, batches grow to
    // put more of them to work at once, and shrink again once they are ahead.
    let fewest = (crate::parallel::threads() / 4).max(4);
    let most = crate::parallel::threads().saturating_mul(2).max(fewest);
    let mut batch_blocks = fewest;
    let mut applying = None;
    let mut poller = state.control.poller();
    let control = state.control;
    let spares = SpareBlocks::default();
    let spares = &spares;
    std::thread::scope(|scope| {
        let mut ready: Option<DecodedBatch<B>> = None;
        let mut finished = false;
        loop {
            let decoding = (!finished).then(|| {
                let batch = read_batch(input, allowance, batch_blocks, || match &ready {
                    // Tables the batch before will have reached by then.
                    Some(previous) => previous.groups.last().and_then(|group| {
                        group
                            .tables
                            .as_ref()
                            .and_then(|tables| tables.try_clone().ok())
                    }),
                    None => state.tables.take(),
                });
                finished = batch.finished;
                scope.spawn(move || decode_batch(batch, algorithm_version, control, spares))
            });
            if let Some(batch) = ready.take() {
                let began = std::time::Instant::now();
                let ended = apply_batch(batch, state, &mut poller, output, output_size, spares)?;
                if let Some(ended) = ended {
                    return Ok(ended);
                }
                applying = Some(began.elapsed());
            }
            let began = std::time::Instant::now();
            ready = decoding.map(|decoding| decoding.join().expect("decoding does not panic"));
            // Clearly behind, not just by timing noise, before taking more memory.
            if let Some(applying) = applying {
                batch_blocks = if began.elapsed() > applying * 2 {
                    batch_blocks.saturating_mul(2).min(most)
                } else {
                    (batch_blocks / 2).max(fewest)
                };
            }
            if ready.is_none() {
                // Every batch ends the member, and the last one has been applied.
                return Err(O::Error::from(Error::NeedMoreInput));
            }
        }
    })
}

#[cfg(all(test, feature = "write"))]
mod tests {
    use super::*;

    /// Data with literals, near and far matches and enough entropy for many blocks.
    fn mixed_data(size: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(size);
        let mut seed = 0x1234_5678u32;
        while data.len() < size {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            match seed >> 30 {
                0 if data.len() > 70_000 => {
                    let start = data.len() - 1 - (seed as usize >> 8) % 70_000;
                    let length = 4 + (seed as usize >> 4) % 300;
                    for index in 0..length {
                        data.push(data[start + index]);
                    }
                }
                1 => data.extend_from_slice(&seed.to_le_bytes()),
                _ => data.extend_from_slice(b"threaded decoder payload "),
            }
        }
        data.truncate(size);
        data
    }

    fn packed(data: &[u8]) -> Vec<u8> {
        encode_lz_member_with_options(
            data,
            0,
            EncodeOptions::new(16).with_max_match_distance(4 * 1024 * 1024),
        )
        .unwrap()
    }

    fn sequential_decode(data: &[u8], size: usize) -> Result<Vec<u8>> {
        // A limited reader never takes the threaded path.
        let allowance = Allowance::limited(u64::MAX / 2);
        ReaderState::new(&allowance)
            .decode_member_from_reader_with_dictionary(
                &mut &data[..],
                0,
                size,
                8 * 1024 * 1024,
                false,
                DecodeMode::Lz,
            )
            .map(|output| output.to_vec())
    }

    fn threaded_decode(data: &[u8], size: usize) -> Result<Vec<u8>> {
        Unpack50Decoder::new().decode_member_from_reader_with_dictionary(
            &mut &data[..],
            0,
            size,
            8 * 1024 * 1024,
            false,
            DecodeMode::Lz,
        )
    }

    fn streamed_decode(data: &[u8], size: usize) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        Unpack50Decoder::new()
            .decode_member_from_reader_with_dictionary_to_sink(
                &mut &data[..],
                0,
                size,
                8 * 1024 * 1024,
                false,
                |chunk| {
                    match chunk {
                        DecodedChunk::Bytes(bytes) => output.extend_from_slice(bytes),
                        DecodedChunk::Repeated { byte, len } => {
                            output.resize(output.len() + len, byte);
                        }
                    }
                    Ok::<_, std::convert::Infallible>(())
                },
            )
            .map_err(|error| match error {
                StreamDecodeError::Decode(error) => error,
                other => panic!("unexpected stream failure: {other:?}"),
            })?;
        Ok(output)
    }

    #[test]
    fn both_outputs_match_the_sequential_decoder() {
        let data = mixed_data(3 * 1024 * 1024);
        let stream = packed(&data);
        assert_eq!(sequential_decode(&stream, data.len()).unwrap(), data);
        assert_eq!(threaded_decode(&stream, data.len()).unwrap(), data);
        assert_eq!(streamed_decode(&stream, data.len()).unwrap(), data);
    }

    #[test]
    fn nothing_past_the_declared_size_can_fail_the_member() {
        let data = mixed_data(2 * 1024 * 1024);
        let mut stream = packed(&data);
        // Whatever the stream holds after the output is complete goes unread by
        // the sequential decoder, so read-ahead must not trip over it either. A
        // size inside a match fails in both, so what matters is that they agree.
        stream.extend_from_slice(&[0xff; 64]);
        assert_eq!(threaded_decode(&stream, data.len()).unwrap(), data);
        for short in 1..=200 {
            let size = data.len() - short * 3000;
            assert_eq!(
                threaded_decode(&stream, size).ok(),
                sequential_decode(&stream, size).ok(),
                "declared {size}"
            );
        }
    }

    #[test]
    fn damage_fails_or_decodes_the_same_as_the_sequential_decoder() {
        let data = mixed_data(2 * 1024 * 1024);
        let stream = packed(&data);
        for cut in [10, stream.len() / 3, stream.len() / 2, stream.len() - 1] {
            let short = &stream[..cut];
            assert_eq!(
                threaded_decode(short, data.len()).ok(),
                sequential_decode(short, data.len()).ok(),
                "truncated at {cut}"
            );
            assert!(threaded_decode(short, data.len()).is_err());
            assert!(streamed_decode(short, data.len()).is_err());
        }
        for at in (0..stream.len()).step_by(stream.len() / 40) {
            let mut bad = stream.clone();
            bad[at] ^= 0x55;
            let expected = sequential_decode(&bad, data.len());
            let threaded = threaded_decode(&bad, data.len());
            let streamed = streamed_decode(&bad, data.len());
            assert_eq!(threaded.is_ok(), expected.is_ok(), "flipped at {at}");
            assert_eq!(streamed.is_ok(), expected.is_ok(), "flipped at {at}");
            if let (Ok(expected), Ok(threaded)) = (&expected, &threaded) {
                assert_eq!(threaded, expected, "flipped at {at}");
            }
        }
    }

    #[test]
    fn solid_members_keep_state_across_both_paths() {
        let first = mixed_data(2 * 1024 * 1024);
        let mut joined = first.clone();
        joined.extend_from_slice(&first[..1024 * 1024]);
        let options = EncodeOptions::new(16).with_max_match_distance(4 * 1024 * 1024);
        let first_packed = encode_lz_member_with_options(&first, 0, options).unwrap();
        let second_packed = {
            let mut encoder = Unpack50Encoder::with_options(options);
            encoder.encode_member(&first, 0).unwrap();
            encoder.encode_member(&first[..1024 * 1024], 0).unwrap()
        };
        let mut decoder = Unpack50Decoder::new();
        let dictionary = 8 * 1024 * 1024;
        for (stream, solid, expected) in [
            (&first_packed, false, &first[..]),
            (&second_packed, true, &first[..1024 * 1024]),
        ] {
            let decoded = decoder
                .decode_member_from_reader_with_dictionary(
                    &mut &stream[..],
                    0,
                    expected.len(),
                    dictionary,
                    solid,
                    DecodeMode::Lz,
                )
                .unwrap();
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn a_cancelled_read_stops_the_member() {
        let data = mixed_data(2 * 1024 * 1024);
        let stream = packed(&data);
        let cancellation = crate::ReadCancellation::new();
        cancellation.cancel();
        let mut decoder = Unpack50Decoder::new();
        decoder.read_control = crate::read_control::ReadControl::new(Some(&cancellation));
        assert!(matches!(
            decoder.decode_member_from_reader_with_dictionary(
                &mut &stream[..],
                0,
                data.len(),
                8 * 1024 * 1024,
                false,
                DecodeMode::Lz,
            ),
            Err(Error::Cancelled)
        ));
    }
}
