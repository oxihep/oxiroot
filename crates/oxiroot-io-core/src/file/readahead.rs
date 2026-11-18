//! Read-ahead for remote files: turn a reader's many small range requests
//! into a few batched ones.
//!
//! Reading a branch or a column touches one range per basket or page. Over a
//! network each range is a round trip, and for a branch of hundreds of baskets
//! the waiting dwarfs the transfer. A reader that knows its ranges up front
//! announces them ([`ByteSource::plan_reads`]); when the first planned range is
//! asked for, it is fetched together with the ranges after it in one
//! [`ByteSource::read_many`] (an XRootD `kXR_readv`, or concurrent HTTP range
//! requests), and the others wait in a small cache until their decoder asks.
//!
//! Each batch is bounded ([`MAX_BATCH_RANGES`], [`MAX_BATCH_BYTES`]), so a
//! plan covering a whole large branch is fetched a batch at a time and the
//! cache never holds more than one batch.

use std::collections::{HashMap, VecDeque};

use bytes::Bytes;

use super::source::ByteSource;
use crate::error::Result;

/// The most ranges one batch fetches (also XRootD's `kXR_readv` limit).
pub(crate) const MAX_BATCH_RANGES: usize = 1024;
/// The most bytes one batch fetches (a batch always holds at least one range).
pub(crate) const MAX_BATCH_BYTES: usize = 32 << 20;

/// The ranges a reader announced, and the fetched ones not yet asked for.
#[derive(Debug, Default)]
pub(crate) struct ReadAhead {
    pending: VecDeque<(u64, usize)>,
    ready: HashMap<(u64, usize), Bytes>,
}

impl ReadAhead {
    /// Replace the plan with `ranges`, in the order they will be read. What a
    /// previous plan fetched but nobody read is dropped.
    pub(crate) fn plan(&mut self, ranges: &[(u64, usize)]) {
        self.ready.clear();
        self.pending = ranges.iter().copied().filter(|&(_, len)| len > 0).collect();
    }

    /// The bytes of `(offset, len)` if the plan has them or covers them —
    /// fetching the batch that starts there — or `None` for a range outside
    /// the plan (the caller reads it directly).
    pub(crate) fn take(
        &mut self,
        source: &dyn ByteSource,
        offset: u64,
        len: usize,
    ) -> Result<Option<Bytes>> {
        if let Some(hit) = self.ready.remove(&(offset, len)) {
            return Ok(Some(hit));
        }
        let Some(at) = self.pending.iter().position(|&r| r == (offset, len)) else {
            return Ok(None);
        };
        // Ranges planned before this one were skipped (read some other way,
        // or not at all): drop them, and fetch from here on.
        self.pending.drain(..at);
        let mut batch = Vec::new();
        let mut bytes = 0usize;
        while let Some(&(o, l)) = self.pending.front() {
            if !batch.is_empty()
                && (batch.len() == MAX_BATCH_RANGES || bytes.saturating_add(l) > MAX_BATCH_BYTES)
            {
                break;
            }
            batch.push((o, l));
            bytes = bytes.saturating_add(l);
            self.pending.pop_front();
        }
        let fetched = source.read_many(&batch)?;
        let mut first = None;
        for (range, data) in batch.into_iter().zip(fetched) {
            if first.is_none() && range == (offset, len) {
                first = Some(data);
            } else {
                self.ready.insert(range, data);
            }
        }
        Ok(first)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::file::source::BytesSource;

    /// A source that counts calls, to see what was batched.
    #[derive(Debug)]
    struct Counting {
        inner: BytesSource,
        calls: Mutex<Vec<usize>>,
    }

    impl ByteSource for Counting {
        fn len(&self) -> u64 {
            self.inner.len()
        }
        fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
            self.calls.lock().unwrap().push(1);
            self.inner.read_at(offset, len)
        }
        fn read_many(&self, ranges: &[(u64, usize)]) -> Result<Vec<Bytes>> {
            self.calls.lock().unwrap().push(ranges.len());
            ranges
                .iter()
                .map(|&(o, l)| self.inner.read_at(o, l))
                .collect()
        }
    }

    fn source() -> Counting {
        Counting {
            inner: BytesSource::new((0..=255u8).cycle().take(4096).collect::<Vec<_>>()),
            calls: Mutex::default(),
        }
    }

    #[test]
    fn planned_ranges_arrive_in_one_batch() {
        let src = source();
        let mut ra = ReadAhead::default();
        let ranges = [(0, 10), (100, 20), (1000, 5)];
        ra.plan(&ranges);
        for &(o, l) in &ranges {
            let got = ra.take(&src, o, l).unwrap().unwrap();
            assert_eq!(got, src.inner.read_at(o, l).unwrap());
        }
        assert_eq!(*src.calls.lock().unwrap(), [3], "one batch of three");
        // Unplanned ranges are left to the caller.
        assert!(ra.take(&src, 7, 3).unwrap().is_none());
    }

    #[test]
    fn a_skipped_range_is_dropped_and_batches_are_bounded() {
        let src = source();
        let mut ra = ReadAhead::default();
        // More ranges than one batch takes.
        let ranges: Vec<(u64, usize)> = (0..MAX_BATCH_RANGES as u64 + 10)
            .map(|i| (i % 4000, 1))
            .collect();
        ra.plan(&ranges);
        // Start at the third: the first two are dropped.
        assert!(ra.take(&src, ranges[2].0, 1).unwrap().is_some());
        assert_eq!(src.calls.lock().unwrap()[0], MAX_BATCH_RANGES);
        assert_eq!(ra.pending.len(), 8);
        // A new plan forgets what the old one fetched.
        ra.plan(&[(0, 4)]);
        assert!(ra.ready.is_empty());
    }
}
