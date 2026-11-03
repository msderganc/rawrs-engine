//! The threaded path's output: one ring holding the history and the bytes not yet
//! handed on, so every byte is written once and a match never pays for finding its
//! source in two buffers. Only an unlimited reader comes here, and the ring gives its
//! history back in order when the member ends or a block needs the sequential decoder.
use super::*;

/// How far a short match may write past its end. Bytes that far ahead are older
/// than any legal distance, since the ring is larger than the dictionary by more.
const SHORT_COPY: usize = 16;
/// The least growth, and the most a declared size alone can ask for beyond doubling.
const MIN_GROWTH: usize = 16 * 1024 * 1024;

pub(super) struct Window<B: Budget> {
    bytes: Buffer<u8, B>,
    /// Where the next byte goes. Everything from `flushed` up to it is unsent.
    position: usize,
    flushed: usize,
    /// How many bytes back the contents reach.
    stored: usize,
    wrapped: bool,
    capacity: usize,
    /// The history this member began with, which zero runs count on.
    inherited: usize,
    stage: FilterStage<B>,
    written: usize,
    output_limit: usize,
    dictionary_size: usize,
    all_zero: bool,
}

impl<B: Budget> Window<B> {
    /// Takes over everything the streaming output holds.
    pub(super) fn take(output: &mut StreamingOutput<B>) -> Self {
        debug_assert!(output.pending.is_empty());
        let allowance = output.history.allowance();
        let history = std::mem::replace(
            &mut output.history,
            super::super::workspace::Deque::from_buffer(Buffer::new(&allowance)),
        )
        .into_buffer();
        let position = history.len();
        Self {
            bytes: history,
            position,
            flushed: position,
            stored: position,
            wrapped: false,
            capacity: output.history_limit.saturating_add(STREAM_FLUSH_THRESHOLD),
            inherited: position,
            stage: std::mem::replace(&mut output.stage, FilterStage::new(&allowance)),
            written: output.written,
            output_limit: output.output_limit,
            dictionary_size: output.dictionary_size,
            all_zero: output.all_zero,
        }
    }

    /// Hands everything back in order. The caller has flushed.
    pub(super) fn give_back(mut self, output: &mut StreamingOutput<B>) -> Result<()> {
        debug_assert_eq!(self.position, self.flushed);
        let limit = output.history_limit;
        if self.wrapped {
            // The oldest bytes worth keeping sit just past the spare room ahead of the
            // write position, so one rotation leaves the rest at the end to drop.
            let spare = self.bytes.len() - limit;
            let oldest = (self.position + spare) % self.bytes.len();
            self.bytes.rotate_left(oldest);
            self.bytes.resize(limit, 0)?;
        } else {
            self.bytes.resize(self.position, 0)?;
            let excess = self.bytes.len().saturating_sub(limit);
            if excess != 0 {
                self.bytes.discard_prefix(excess);
            }
        }
        output.history = super::super::workspace::Deque::from_buffer(self.bytes);
        output.stage = self.stage;
        output.written = self.written;
        output.all_zero = self.all_zero;
        Ok(())
    }

    pub(super) fn written(&self) -> usize {
        self.written
    }

    pub(super) fn flush<E>(
        &mut self,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        if self.flushed != self.position {
            let pending = &self.bytes[self.flushed..self.position];
            self.stage.emit(DecodedChunk::Bytes(pending), sink)?;
            self.flushed = self.position;
        }
        Ok(())
    }

    /// Makes room for at least one byte and returns how much there is in a row.
    fn room<E>(
        &mut self,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<usize, StreamDecodeError<E>> {
        if self.position == self.bytes.len() {
            if self.bytes.len() < self.capacity {
                // Enough for this member, as filling more than it uses is wasted work
                // for every member of a solid archive. A declared size can lie, so
                // it only gets as far as doubling would.
                let unwritten = (self.output_limit - self.written).max(1);
                let grown = self
                    .position
                    .saturating_add(unwritten.min(self.position.max(MIN_GROWTH)))
                    .min(self.capacity);
                // Room for it all at once where that is modest, as growing in steps
                // holds the old and new rings together while it copies.
                let wanted = self
                    .position
                    .saturating_add(unwritten)
                    .min(self.capacity)
                    .min(MAX_RESERVED_OUTPUT);
                self.bytes.reserve_total_capacity(wanted.max(grown))?;
                self.bytes.resize(grown, 0)?;
            } else {
                self.flush(sink)?;
                self.position = 0;
                self.flushed = 0;
                self.wrapped = true;
            }
        }
        Ok(self.bytes.len() - self.position)
    }

    fn check_limit<E>(&self, count: usize) -> std::result::Result<(), StreamDecodeError<E>> {
        if self
            .written
            .checked_add(count)
            .is_none_or(|end| end > self.output_limit)
        {
            return Err(Error::InvalidData("RAR 5 match exceeds output limit").into());
        }
        Ok(())
    }

    #[inline]
    fn advance<E>(
        &mut self,
        count: usize,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        self.position += count;
        self.written += count;
        self.stored += count;
        if self.position - self.flushed >= STREAM_FLUSH_THRESHOLD {
            self.flush(sink)?;
        }
        Ok(())
    }

    pub(super) fn literals<E>(
        &mut self,
        mut bytes: &[u8],
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        self.check_limit(bytes.len())?;
        if self.all_zero && bytes.iter().any(|&byte| byte != 0) {
            self.all_zero = false;
        }
        while !bytes.is_empty() {
            let take = bytes.len().min(self.room(sink)?);
            let (now, rest) = bytes.split_at(take);
            self.bytes[self.position..self.position + take].copy_from_slice(now);
            self.advance(take, sink)?;
            bytes = rest;
        }
        Ok(())
    }

    fn fill<E>(
        &mut self,
        byte: u8,
        mut count: usize,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        if byte != 0 {
            self.all_zero = false;
        }
        while count != 0 {
            let take = count.min(self.room(sink)?);
            self.bytes[self.position..self.position + take].fill(byte);
            self.advance(take, sink)?;
            count -= take;
        }
        Ok(())
    }

    fn byte_before(&self) -> u8 {
        if self.position != 0 {
            self.bytes[self.position - 1]
        } else {
            self.bytes[self.bytes.len() - 1]
        }
    }

    #[inline]
    pub(super) fn copy_match<E>(
        &mut self,
        distance: usize,
        length: usize,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        let position = self.position;
        // Most matches are short and far enough back not to overlap, and copying a
        // fixed size is much cheaper than a call that handles any size.
        if length <= SHORT_COPY
            && distance >= SHORT_COPY
            && distance <= position
            && distance <= self.dictionary_size
            && position + SHORT_COPY <= self.bytes.len()
            && !self.all_zero
            && self.written + length <= self.output_limit
        {
            let (before, after) = self.bytes.split_at_mut(position);
            let start = position - distance;
            after[..SHORT_COPY].copy_from_slice(&before[start..start + SHORT_COPY]);
            return self.advance(length, sink);
        }
        self.copy_long(distance, length, sink)
    }

    fn copy_long<E>(
        &mut self,
        distance: usize,
        length: usize,
        sink: &mut impl FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), StreamDecodeError<E>> {
        self.check_limit(length)?;
        if self.all_zero && distance <= self.written + self.inherited {
            // A run of zeroes costs no memory, however long.
            self.flush(sink)?;
            self.stage.emit(
                DecodedChunk::Repeated {
                    byte: 0,
                    len: length,
                },
                sink,
            )?;
            self.written += length;
            return Ok(());
        }
        // Zero-fill out-of-window matches, as the buffered decoder does.
        if distance == 0 || distance > self.dictionary_size || distance > self.stored {
            return self.fill(0, length, sink);
        }
        if distance == 1 {
            let byte = self.byte_before();
            return self.fill(byte, length, sink);
        }
        let mut remaining = length;
        // Bytes a match has written repeat its period, so any whole number of
        // periods back reads the same. Short overlapping matches take
        // logarithmically many copies by reaching further each time.
        let mut period = distance;
        while remaining != 0 {
            let room = self.room(sink)?;
            let position = self.position;
            if period > position {
                period = distance;
            }
            let (start, reach) = if period <= position {
                (position - period, period)
            } else {
                let start = self.bytes.len() - (distance - position);
                (start, self.bytes.len() - start)
            };
            let take = remaining.min(room).min(reach);
            self.bytes.copy_within(start..start + take, position);
            self.advance(take, sink)?;
            remaining -= take;
            if take == period {
                period *= 2;
            }
        }
        Ok(())
    }
}

/// The streaming decoder's end of the threaded path's ordered stage.
pub(super) struct WindowRecords<'a, 'f, B: Budget, S, E> {
    pub(super) window: &'a mut Window<B>,
    pub(super) sink: &'a mut S,
    pub(super) filters:
        &'a mut Option<&'f mut dyn FnMut(PendingFilter) -> std::result::Result<(), E>>,
    pub(super) output_size: usize,
}

impl<B: Budget, S, E> threaded::RecordOutput for WindowRecords<'_, '_, B, S, E>
where
    S: FnMut(DecodedChunk<'_>) -> std::result::Result<(), E>,
{
    type Error = StreamDecodeError<E>;
    #[inline]
    fn written(&self) -> usize {
        self.window.written()
    }
    fn literals(&mut self, bytes: &[u8]) -> std::result::Result<(), Self::Error> {
        self.window.literals(bytes, self.sink)
    }
    #[inline]
    fn copy_match(
        &mut self,
        distance: usize,
        length: usize,
    ) -> std::result::Result<(), Self::Error> {
        self.window.copy_match(distance, length, self.sink)
    }
    fn filter(&mut self, mut filter: PendingFilter) -> std::result::Result<(), Self::Error> {
        filter.start = filter
            .start
            .checked_add(self.window.written())
            .ok_or(Error::InvalidData("RAR 5 filter start overflows"))?;
        if filter
            .start
            .checked_add(filter.length)
            .is_none_or(|end| end > self.output_size)
        {
            return Err(Error::InvalidData("RAR 5 filter range exceeds output").into());
        }
        match self.filters.as_mut() {
            Some(filters) => filters(filter).map_err(StreamDecodeError::Sink),
            None => self.window.stage.hold(filter),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Literals and matches of every kind, in an order the seed decides.
    fn run_both(seed: u32, history: &[u8], dictionary: usize, size: usize) {
        let collect = |into: &mut Vec<u8>, chunk: DecodedChunk<'_>| match chunk {
            DecodedChunk::Bytes(bytes) => into.extend_from_slice(bytes),
            DecodedChunk::Repeated { byte, len } => into.resize(into.len() + len, byte),
        };
        let mut plain = Vec::new();
        let mut through = Vec::new();
        let new_output = || {
            StreamingOutput::new(
                Buffer::from_vec(history.to_vec()),
                size,
                dictionary,
                dictionary,
            )
            .unwrap()
        };
        let mut old = new_output();
        let mut new = new_output();
        let mut window = Window::take(&mut new);
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        let mut old_sink = |chunk: DecodedChunk<'_>| {
            collect(&mut plain, chunk);
            Ok::<(), ()>(())
        };
        let mut new_sink = |chunk: DecodedChunk<'_>| {
            collect(&mut through, chunk);
            Ok::<(), ()>(())
        };
        let mut done = 0;
        while done < size {
            let room = size - done;
            let pick = next() as usize;
            let length = match pick % 8 {
                0 => 1 + pick % 8,
                1..=3 => 2 + pick % 15,
                4..=6 => 17 + pick % 400,
                _ => 1000 + pick % (3 * dictionary).max(1),
            }
            .min(room);
            if pick % 11 < 4 {
                let literals: Vec<u8> = (0..length.min(300)).map(|_| next() as u8).collect();
                let mut one = literals.as_slice();
                while let Some((&byte, rest)) = one.split_first() {
                    old.push(byte, &mut old_sink).unwrap();
                    one = rest;
                }
                window.literals(&literals, &mut new_sink).unwrap();
                done += literals.len();
            } else {
                let distance = match next() % 5 {
                    0 => 1 + next() as usize % 3,
                    1 => 2 + next() as usize % 40,
                    2 => dictionary,
                    3 => dictionary + 1,
                    _ => 1 + next() as usize % dictionary,
                };
                old.copy_match(distance, length, &mut old_sink).unwrap();
                window.copy_match(distance, length, &mut new_sink).unwrap();
                done += length;
            }
            assert_eq!(old.written(), window.written());
        }
        old.finish(&mut old_sink).unwrap();
        window.flush(&mut new_sink).unwrap();
        window.give_back(&mut new).unwrap();
        new.finish(&mut new_sink).unwrap();
        assert_eq!(plain, through, "seed {seed}");
        assert_eq!(new.written(), size);
        assert_eq!(&*old.into_history(), &*new.into_history(), "seed {seed}");
    }

    #[test]
    fn matches_the_streaming_output_through_wraps_and_overlaps() {
        // The ring is the dictionary plus one flush, so these all wrap several times.
        for seed in 0..16 {
            run_both(seed, &[], 4 * 1024, 400 * 1024);
            run_both(seed, b"history before this member", 64, 300 * 1024);
            run_both(seed, &vec![7; 32 * 1024], 32 * 1024, 500 * 1024);
        }
    }

    #[test]
    fn matches_the_streaming_output_before_it_fills() {
        for seed in 0..16 {
            run_both(seed, b"short", 8 * 1024 * 1024, 200 * 1024);
        }
    }

    #[test]
    fn zero_runs_stay_out_of_the_ring_until_something_else_arrives() {
        let mut output = StreamingOutput::new(Buffer::from_vec(vec![0]), 3 << 30, 16, 16).unwrap();
        let mut window = Window::take(&mut output);
        let mut total = 0usize;
        let mut sink = |chunk: DecodedChunk<'_>| {
            total += chunk_len(chunk);
            Ok::<(), ()>(())
        };
        window.copy_match(1, 2 << 30, &mut sink).unwrap();
        assert!(window.bytes.len() <= 1);
        window.literals(b"x", &mut sink).unwrap();
        window.copy_match(1, 4, &mut sink).unwrap();
        window.flush(&mut sink).unwrap();
        assert_eq!(total, (2 << 30) + 5);
    }
}
