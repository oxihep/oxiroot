//! Multithreaded histogram fill — the pure-Rust analog of ROOT's
//! [`TThreadedObject`](https://root.cern/doc/master/classROOT_1_1TThreadedObject.html).
//!
//! ROOT fills histograms across threads by giving each thread a private copy and
//! merging them at the end. oxiroot is set up for exactly this: every histogram
//! type is [`Clone`] plain data, and [`add(other, 1.0)`](crate::Hist1D::add) is an
//! *exact* reduction — it combines bin contents, per-bin `Sumw2`, the entry
//! count, and every statistical moment sum. So a parallel fill is just "clone per
//! thread, fill locally without locking, merge at the end", and the merged
//! histogram is identical to a serial fill (up to floating-point summation order).
//!
//! - [`Mergeable`] — the reduction trait (`merge` == `add(other, 1.0)`); its
//!   [`merge_all`](Mergeable::merge_all) folds an iterator of histograms into one
//!   (an in-memory `hadd`).
//! - [`ThreadedHist`] — the accumulator: share `&ThreadedHist`, call
//!   [`fill`](ThreadedHist::fill) from any thread (each gets its own copy), then
//!   [`merge`](ThreadedHist::merge) at the end. Works with [`std::thread::scope`].
//! - [`fill_par`] — one-call parallel fill of a slice (requires the `rayon`
//!   feature).

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use thread_local::ThreadLocal;

use oxiroot_io_core::{Parameter, Result};

use crate::{
    Efficiency, Graph, Hist1D, Hist2D, Hist3D, PolyHist, Profile1D, Profile2D, Profile3D,
    SparseHist,
};

/// Histograms that combine into one — the reduction behind in-memory merges
/// (multithreaded fills, [`ThreadedHist`], `fill_par`). The file merger
/// (`hadd`) sums histograms with `add` directly.
///
/// [`merge`](Mergeable::merge) is the bin-by-bin combine of `add(other, 1.0)`: it
/// sums contents, per-bin `Sumw2`, the entry count, and every moment sum, so the
/// result is identical to having filled one histogram with all the data. It
/// returns [`oxiroot_io_core::Error::BinningMismatch`] (leaving `self` unchanged)
/// if the binnings differ.
#[doc(alias = "Merge")]
pub trait Mergeable: Clone + Send + Sized {
    /// Combine `other` into `self` (the `c == 1` case of `add`).
    fn merge(&mut self, other: &Self) -> Result<()>;

    /// Fold an iterator of histograms into one, starting from the first item.
    /// Returns `Ok(None)` for an empty iterator, or the binning-mismatch error
    /// from the first incompatible pair.
    fn merge_all<I: IntoIterator<Item = Self>>(items: I) -> Result<Option<Self>> {
        let mut it = items.into_iter();
        let Some(mut acc) = it.next() else {
            return Ok(None);
        };
        for h in it {
            acc.merge(&h)?;
        }
        Ok(Some(acc))
    }
}

macro_rules! impl_mergeable {
    ($($t:ty),+ $(,)?) => {$(
        impl Mergeable for $t {
            fn merge(&mut self, other: &Self) -> Result<()> {
                self.add(other, 1.0)
            }
        }
    )+};
}
impl_mergeable!(Hist1D, Hist2D, Hist3D, Profile1D, Profile2D, Profile3D, PolyHist, SparseHist);

impl Mergeable for Efficiency {
    fn merge(&mut self, other: &Self) -> Result<()> {
        self.add(other)
    }
}

impl Mergeable for Graph {
    /// Appends `other`'s points, the way ROOT's `hadd` merges graphs.
    fn merge(&mut self, other: &Self) -> Result<()> {
        self.append(other)
    }
}

impl Mergeable for Parameter {
    /// Adds the two values, the way ROOT's `hadd` merges parameters.
    fn merge(&mut self, other: &Self) -> Result<()> {
        self.add(other)
    }
}

/// A multithreaded fill accumulator — the pure-Rust analog of ROOT's
/// `TThreadedObject<Hist1D>`.
///
/// Hold one *template* histogram (a binning prototype, normally **empty**), share
/// `&ThreadedHist` across threads, and call [`fill`](ThreadedHist::fill) from any
/// of them: each thread transparently gets its own private copy of the template
/// (created on first use) and fills it without touching shared state. At the
/// end, [`merge`](Self::merge) combines every thread's copy into one — exactly as
/// if a single histogram had been filled with all the data (up to floating-point
/// summation order). This mirrors ROOT's `TThreadedObject` (`Fill` per thread,
/// `Merge` at the end), without ROOT's explicit slot bookkeeping.
///
/// `&ThreadedHist` is [`Sync`], so it is shared across [`std::thread::scope`]
/// workers without an `Arc`. Finding a thread's copy takes no shared lock (the
/// copies live in thread-local storage), so fills scale with the number of
/// threads.
///
/// ```
/// use oxiroot_hist::ToRoot;
/// use oxiroot_hist::{Hist, ThreadedHist};
///
/// let data: Vec<f64> = (0..1000).map(|i| i as f64 % 100.0).collect();
/// let hist = ThreadedHist::new(Hist::reg(100, 0.0, 100.0).double().to_root().named("h"));
///
/// std::thread::scope(|s| {
///     for chunk in data.chunks(data.len().div_ceil(4)) {
///         let hist = &hist;
///         s.spawn(move || {
///             for &x in chunk {
///                 hist.fill(x); // each thread fills its own copy — no setup
///             }
///         });
///     }
/// });
///
/// let merged = hist.merge().unwrap(); // combine every thread's copy
/// assert_eq!(merged.entries, 1000.0);
/// ```
pub struct ThreadedHist<H: Mergeable> {
    template: H,
    /// One copy per thread. `ThreadLocal` finds the calling thread's slot
    /// without a shared lock, so filling threads never write to a common cache
    /// line (a shared `RwLock` here made fills slower with more threads).
    slots: ThreadLocal<RefCell<H>>,
    count: AtomicUsize,
}

impl<H: Mergeable> ThreadedHist<H> {
    /// Create an accumulator from a template histogram — a binning prototype,
    /// normally empty. Each thread's private copy is a clone of it.
    pub fn new(template: H) -> Self {
        Self {
            template,
            slots: ThreadLocal::new(),
            count: AtomicUsize::new(0),
        }
    }

    /// Run `f` on the calling thread's private copy of the histogram, creating it
    /// (a clone of the template) on first use. This is the generic primitive
    /// behind [`fill`](ThreadedHist::fill): use it to call any method, or to fill
    /// a whole batch under a single slot acquisition:
    ///
    /// ```
    /// use oxiroot_hist::ToRoot;
    /// # use oxiroot_hist::{Hist, ThreadedHist};
    /// # let hist = ThreadedHist::new(Hist::reg(10, 0.0, 1.0).double().to_root().named("h"));
    /// hist.with_local(|h| {
    ///     for x in [0.1, 0.2, 0.3] {
    ///         h.fill(x);
    ///     }
    /// });
    /// ```
    ///
    /// # Panics
    ///
    /// If `f` uses the same accumulator again (a nested `fill` or
    /// `with_local`): the thread's copy is already borrowed.
    pub fn with_local<R>(&self, f: impl FnOnce(&mut H) -> R) -> R {
        let slot = self.slots.get_or(|| {
            self.count.fetch_add(1, Ordering::Relaxed);
            RefCell::new(self.template.clone())
        });
        f(&mut slot.borrow_mut())
    }

    /// The number of per-thread copies made so far: at most one per thread
    /// that filled. A thread started after another has exited may reuse the
    /// exited thread's copy, which keeps that thread's fills, so the merged
    /// result is unaffected.
    pub fn num_slots(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Combine every thread's private copy into the final histogram, consuming
    /// the accumulator. Returns an (empty) clone of the template if no thread
    /// filled. Call after the workers have joined. Errors only if a copy's
    /// binning diverged from the template (normal fills keep it).
    pub fn merge(self) -> Result<H> {
        let locals = self.slots.into_iter().map(RefCell::into_inner);
        Ok(H::merge_all(locals)?.unwrap_or(self.template))
    }
}

impl ThreadedHist<Hist1D> {
    /// Fill the calling thread's copy with `x` (weight 1).
    pub fn fill(&self, x: f64) {
        self.with_local(|h| h.fill(x));
    }
    /// Fill the calling thread's copy with `x` and weight `w`.
    pub fn fill_weight(&self, x: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, w));
    }
}

impl ThreadedHist<Hist2D> {
    /// Fill the calling thread's copy at `(x, y)` (weight 1).
    pub fn fill(&self, x: f64, y: f64) {
        self.with_local(|h| h.fill(x, y));
    }
    /// Fill the calling thread's copy at `(x, y)` with weight `w`.
    pub fn fill_weight(&self, x: f64, y: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, y, w));
    }
}

impl ThreadedHist<Hist3D> {
    /// Fill the calling thread's copy at `(x, y, z)` (weight 1).
    pub fn fill(&self, x: f64, y: f64, z: f64) {
        self.with_local(|h| h.fill(x, y, z));
    }
    /// Fill the calling thread's copy at `(x, y, z)` with weight `w`.
    pub fn fill_weight(&self, x: f64, y: f64, z: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, y, z, w));
    }
}

impl ThreadedHist<Profile1D> {
    /// Fill the calling thread's copy at `(x, y)` (weight 1).
    pub fn fill(&self, x: f64, y: f64) {
        self.with_local(|h| h.fill(x, y));
    }
    /// Fill the calling thread's copy at `(x, y)` with weight `w`.
    pub fn fill_weight(&self, x: f64, y: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, y, w));
    }
}

impl ThreadedHist<Profile2D> {
    /// Profile `z` at `(x, y)` in the calling thread's copy (weight 1).
    pub fn fill(&self, x: f64, y: f64, z: f64) {
        self.with_local(|h| h.fill(x, y, z));
    }
    /// Profile `z` at `(x, y)` in the calling thread's copy with weight `w`.
    pub fn fill_weight(&self, x: f64, y: f64, z: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, y, z, w));
    }
}

impl ThreadedHist<Profile3D> {
    /// Profile `t` at `(x, y, z)` in the calling thread's copy (weight 1).
    pub fn fill(&self, x: f64, y: f64, z: f64, t: f64) {
        self.with_local(|h| h.fill(x, y, z, t));
    }
    /// Profile `t` at `(x, y, z)` in the calling thread's copy with weight `w`.
    pub fn fill_weight(&self, x: f64, y: f64, z: f64, t: f64, w: f64) {
        self.with_local(|h| h.fill_weight(x, y, z, t, w));
    }
}

/// Fill a histogram from a slice in parallel (requires the `rayon` feature).
///
/// Convenience over [`ThreadedHist`] for the common "one histogram, fill from a
/// `&[T]`" case: `data` is cut into a few chunks per worker thread, each chunk
/// is filled into one `template.clone()`, and the partial histograms reduce
/// with [`Mergeable::merge`].
/// The result equals a serial fill (up to floating-point summation order).
///
/// `fill(&mut h, &item)` applies one item — e.g. `|h, &x| h.fill(x)` for a 1-D
/// histogram, or `|h, ev| h.fill_weight(ev.x, ev.w)`.
///
/// ```
/// use oxiroot_hist::ToRoot;
/// # #[cfg(feature = "rayon")] {
/// use oxiroot_hist::{fill_par, Hist};
/// let data: Vec<f64> = (0..1000).map(|i| i as f64 % 100.0).collect();
/// let template = Hist::reg(100, 0.0, 100.0).double().to_root().named("h");
/// let hist = fill_par(&template, &data, |h, &x| h.fill(x));
/// assert_eq!(hist.entries, 1000.0);
/// # }
/// ```
#[cfg(feature = "rayon")]
pub fn fill_par<H, T, F>(template: &H, data: &[T], fill: F) -> H
where
    H: Mergeable + Sync,
    T: Sync,
    F: Fn(&mut H, &T) + Sync,
{
    use rayon::prelude::*;
    // A few chunks per thread balance the load without cloning the template
    // once per rayon split (which costs a lot for a large histogram).
    let chunks = rayon::current_num_threads().max(1) * 4;
    let chunk_len = data.len().div_ceil(chunks).max(1);
    data.par_chunks(chunk_len)
        .map(|chunk| {
            let mut h = template.clone();
            for item in chunk {
                fill(&mut h, item);
            }
            h
        })
        .reduce_with(|mut a, b| {
            // Every partial is cloned from the same template, so the binnings
            // are identical and `merge` cannot mismatch here.
            a.merge(&b)
                .expect("fill_par: identical binning by construction");
            a
        })
        .unwrap_or_else(|| template.clone())
}
