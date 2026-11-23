//! Actions on a query: results booked on any query of a tree, and filled
//! together in one pass over the dataset.

use std::any::Any;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};

use oxiroot_io_core::{Error, Result};

use super::check::{Source, check};
use super::eval::Val;
use super::{Expr, Graph, IntoExpr, Planned, Query, QueryError, State};
use crate::Batch;

/// What a [`Query::fill`] can fill: a histogram of `N` dimensions, filled
/// at one point per entry (or per object) with a weight, and merged with
/// the copies other threads filled.
///
/// The histograms of oxihist ([`Hist`](oxihist::Hist) builds them) with a
/// counting storage implement it in 1, 2 and 3 dimensions.
pub trait Fill<const N: usize>: Clone + Send + Sync + 'static {
    /// Add one entry at `x`, with `weight`.
    ///
    /// # Errors
    /// A point the histogram cannot take.
    fn fill_at(&mut self, x: [f64; N], weight: f64) -> Result<()>;
    /// Add everything `other` holds, as if this had been filled with it.
    ///
    /// # Errors
    /// Two histograms that do not combine (different binnings).
    fn merge(&mut self, other: &Self) -> Result<()>;
    /// An empty copy, with the same binning: what a thread starts from.
    #[must_use]
    fn cleared(&self) -> Self;
}

macro_rules! fill_histogram {
    ($n:literal, [$($i:literal),+]) => {
        impl<S: oxihist::CountStorage> Fill<$n> for oxihist::Histogram<S, $n> {
            fn fill_at(&mut self, x: [f64; $n], weight: f64) -> Result<()> {
                self.try_fill_weight($(x[$i]),+, weight).map_err(Error::other)
            }
            fn merge(&mut self, other: &Self) -> Result<()> {
                oxihist::Mergeable::merge(self, other).map_err(Error::other)
            }
            fn cleared(&self) -> Self {
                let mut h = self.clone();
                h.reset();
                h
            }
        }
    };
}
fill_histogram!(1, [0]);
fill_histogram!(2, [0, 1]);
fill_histogram!(3, [0, 1, 2]);

/// How many entries each cut of a query kept, in order: its cut flow,
/// from [`Query::report`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    cuts: Vec<Cut>,
}

/// One cut of a [`Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    name: String,
    all: u64,
    passed: u64,
}

impl Cut {
    /// The name given to [`Query::filter_named`], or the cut's expression.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The entries that reached the cut.
    #[must_use]
    pub const fn all(&self) -> u64 {
        self.all
    }
    /// The entries the cut kept.
    #[must_use]
    pub const fn passed(&self) -> u64 {
        self.passed
    }
    /// The fraction of the entries that reached the cut that it kept, or
    /// `None` if none reached it.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "a fraction of two counts, as a float"
    )]
    pub fn efficiency(&self) -> Option<f64> {
        (self.all > 0).then(|| self.passed as f64 / self.all as f64)
    }
}

impl Report {
    /// The cuts, from the first to the last.
    #[must_use]
    pub fn cuts(&self) -> &[Cut] {
        &self.cuts
    }
    /// The cut named `name`.
    #[must_use]
    pub fn cut(&self, name: &str) -> Option<&Cut> {
        self.cuts.iter().find(|c| c.name == name)
    }
}

/// One line per cut: the entries it kept, of those that reached it, the
/// fraction, and the fraction of the entries before the first cut.
impl std::fmt::Display for Report {
    #[allow(clippy::cast_precision_loss, reason = "fractions of counts, as floats")]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let width = self
            .cuts
            .iter()
            .map(|c| c.name.chars().count())
            .max()
            .unwrap_or(0);
        let start = self.cuts.first().map_or(0, |c| c.all);
        for c in &self.cuts {
            let percent = |n: u64, of: u64| {
                if of == 0 {
                    "    –".to_owned()
                } else {
                    format!("{:5.1}", 100.0 * n as f64 / of as f64)
                }
            };
            writeln!(
                f,
                "{:<width$}  {:>10} of {:>10}  {} %  (cumulative {} %)",
                c.name,
                c.passed,
                c.all,
                percent(c.passed, c.all),
                percent(c.passed, start),
            )?;
        }
        Ok(())
    }
}

/// What a finished action holds, or why it has nothing.
enum Outcome {
    Done(Box<dyn Any + Send>),
    Failed(Error),
}

type Slot = Arc<Mutex<Option<Outcome>>>;

fn fill_slot(slot: &Slot, outcome: Outcome) {
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
}

/// What can run a query's booked actions: its graph.
trait Runner: Send + Sync {
    fn run(&self) -> Result<()>;
}

impl Runner for Mutex<Graph<'_, '_>> {
    fn run(&self) -> Result<()> {
        self.lock().unwrap_or_else(PoisonError::into_inner).run()
    }
}

/// The result of an action booked on a query: a count, a sum, a filled
/// histogram, a cut flow. Nothing is read when it is booked: the first
/// [`get`](Self::get) of any action booked on the same [`Dataset::query`]
/// runs one pass over the dataset that fills them all.
///
/// [`Dataset::query`]: crate::Dataset::query
#[must_use = "an action is filled when its result is asked for, with `get`"]
pub struct Handle<'q, T> {
    runner: Arc<dyn Runner + 'q>,
    slot: Slot,
    kind: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for Handle<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("ready", &self.is_ready())
            .finish_non_exhaustive()
    }
}

impl<T> Handle<'_, T> {
    /// Whether the result is filled (or failed) already, so that
    /// [`get`](Self::get) reads nothing.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

impl<T: 'static> Handle<'_, T> {
    /// The result: filled now, with every other action booked on the same
    /// query, unless an earlier pass filled it already.
    ///
    /// # Errors
    /// A [`QueryError`] (inside [`Error::Other`]) if the action or the
    /// query it is booked on does not check, or a value does not fit; a
    /// read error. A pass that fails leaves every action it was filling
    /// failed.
    pub fn get(self) -> Result<T> {
        if !self.is_ready() {
            self.runner.run()?;
        }
        let outcome = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match outcome {
            Some(Outcome::Done(value)) => Ok(*value
                .downcast::<T>()
                .unwrap_or_else(|_| unreachable!("a slot holds its handle's type"))),
            Some(Outcome::Failed(e)) => Err(e),
            None => Err(Error::other(QueryError::new(
                "the pass that fills this result did not run",
            ))),
        }
    }
}

/// A partial result, filled batch by batch, one per thread, then merged.
trait Acc: Send + Sync {
    /// Add `values` (one per expression of the action) of `entries`
    /// entries.
    fn add(&mut self, values: &[Val], entries: usize) -> std::result::Result<(), String>;
    /// An empty one, for another thread.
    fn fresh(&self) -> Box<dyn Acc>;
    fn merge(&mut self, other: Box<dyn Acc>) -> std::result::Result<(), String>;
    fn finish(self: Box<Self>) -> Box<dyn Any + Send>;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

fn same<T: 'static>(other: Box<dyn Acc>) -> T {
    *other
        .into_any()
        .downcast::<T>()
        .unwrap_or_else(|_| unreachable!("an action merges with its own kind"))
}

/// The values of `values` as rows: one per entry, or one per object where
/// a value has one per object (a per-entry value repeats for each of the
/// entry's objects), as floats; and whether every value of each row is
/// present.
fn rows(values: &[Val], entries: usize) -> std::result::Result<(Vec<Vec<f64>>, Vec<bool>), String> {
    let offsets = values.iter().find_map(|v| v.offsets.clone());
    if let Some(o) = &offsets
        && values
            .iter()
            .filter_map(|v| v.offsets.as_ref())
            .any(|p| p != o)
    {
        return Err(
            "two values with one per object have different numbers of objects in an entry"
                .to_owned(),
        );
    }
    let n = offsets.as_ref().map_or(entries, |o| {
        usize::try_from(*o.last().unwrap_or(&0)).unwrap_or(usize::MAX)
    });
    let mut present = vec![true; n];
    let mut columns = Vec::with_capacity(values.len());
    for v in values {
        let data = v.data.floats();
        match (&offsets, &v.offsets) {
            (Some(o), None) => {
                let mut column = Vec::with_capacity(n);
                for e in 0..entries {
                    let (a, b) = (o[e], o[e + 1]);
                    let ok = v.valid.as_ref().is_none_or(|valid| valid[e]);
                    for r in a..b {
                        let r = usize::try_from(r).unwrap_or(usize::MAX);
                        column.push(data[e]);
                        present[r] &= ok;
                    }
                }
                columns.push(column);
            }
            _ => {
                if let Some(valid) = &v.valid {
                    for (p, &ok) in present.iter_mut().zip(valid) {
                        *p &= ok;
                    }
                }
                columns.push(data);
            }
        }
    }
    Ok((columns, present))
}

/// The present values of one expression, one per entry or per object.
fn present_values(values: &[Val], entries: usize) -> std::result::Result<Vec<f64>, String> {
    let (columns, present) = rows(values, entries)?;
    Ok(columns[0]
        .iter()
        .zip(present)
        .filter_map(|(&x, ok)| ok.then_some(x))
        .collect())
}

struct Count(u64);

impl Acc for Count {
    fn add(&mut self, _: &[Val], entries: usize) -> std::result::Result<(), String> {
        self.0 += entries as u64;
        Ok(())
    }
    fn fresh(&self) -> Box<dyn Acc> {
        Box::new(Self(0))
    }
    fn merge(&mut self, other: Box<dyn Acc>) -> std::result::Result<(), String> {
        self.0 += same::<Self>(other).0;
        Ok(())
    }
    fn finish(self: Box<Self>) -> Box<dyn Any + Send> {
        Box::new(self.0)
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// Sum, count, and the extremes: what `sum`, `mean`, `min` and `max`
/// finish from.
#[derive(Clone, Copy)]
enum Stat {
    Sum,
    Mean,
    Min,
    Max,
}

struct Moments {
    stat: Stat,
    sum: f64,
    n: u64,
    min: f64,
    max: f64,
}

impl Moments {
    const fn new(stat: Stat) -> Self {
        Self {
            stat,
            sum: 0.0,
            n: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl Acc for Moments {
    fn add(&mut self, values: &[Val], entries: usize) -> std::result::Result<(), String> {
        for x in present_values(values, entries)? {
            self.sum += x;
            self.n += 1;
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
        Ok(())
    }
    fn fresh(&self) -> Box<dyn Acc> {
        Box::new(Self::new(self.stat))
    }
    fn merge(&mut self, other: Box<dyn Acc>) -> std::result::Result<(), String> {
        let o = same::<Self>(other);
        self.sum += o.sum;
        self.n += o.n;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        Ok(())
    }
    #[allow(clippy::cast_precision_loss, reason = "a mean, as a float")]
    fn finish(self: Box<Self>) -> Box<dyn Any + Send> {
        let some = self.n > 0;
        match self.stat {
            Stat::Sum => Box::new(self.sum),
            Stat::Mean => Box::new(some.then(|| self.sum / self.n as f64)),
            Stat::Min => Box::new(some.then_some(self.min)),
            Stat::Max => Box::new(some.then_some(self.max)),
        }
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

struct Filled<H, const N: usize> {
    hist: H,
    weighted: bool,
}

impl<H: Fill<N>, const N: usize> Acc for Filled<H, N> {
    fn add(&mut self, values: &[Val], entries: usize) -> std::result::Result<(), String> {
        let (columns, present) = rows(values, entries)?;
        for (r, ok) in present.into_iter().enumerate() {
            if ok {
                let w = if self.weighted { columns[N][r] } else { 1.0 };
                self.hist
                    .fill_at(std::array::from_fn(|k| columns[k][r]), w)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    fn fresh(&self) -> Box<dyn Acc> {
        Box::new(Self {
            hist: self.hist.cleared(),
            weighted: self.weighted,
        })
    }
    fn merge(&mut self, other: Box<dyn Acc>) -> std::result::Result<(), String> {
        self.hist
            .merge(&same::<Self>(other).hist)
            .map_err(|e| e.to_string())
    }
    fn finish(self: Box<Self>) -> Box<dyn Any + Send> {
        Box::new(self.hist)
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// What an action does.
enum What {
    /// Evaluate `exprs` at the action's query and add them to `acc`.
    Acc {
        label: String,
        exprs: Vec<Expr>,
        acc: Box<dyn Acc>,
    },
    /// The cut flow of the action's query.
    Report,
    /// The action's query's output, as `read` gives it.
    Collect,
}

/// An action waiting for the pass that fills it.
pub(super) struct Booked {
    node: usize,
    what: What,
    slot: Slot,
}

impl<'d, 'f> Query<'d, 'f> {
    fn book<T: 'static>(&self, what: std::result::Result<What, QueryError>) -> Handle<'d, T> {
        let slot: Slot = Arc::new(Mutex::new(None));
        match what {
            Ok(what) => self.lock().booked.push(Booked {
                node: self.node,
                what,
                slot: Arc::clone(&slot),
            }),
            Err(e) => fill_slot(&slot, Outcome::Failed(Error::other(e))),
        }
        Handle {
            runner: self.graph.clone(),
            slot,
            kind: PhantomData,
        }
    }

    fn book_stat(&self, label: &str, stat: Stat, expr: impl IntoExpr) -> Handle<'d, Option<f64>> {
        self.book(expr.into_expr().map(|e| What::Acc {
            label: format!("{label}({e})"),
            exprs: vec![e],
            acc: Box::new(Moments::new(stat)),
        }))
    }

    /// The number of entries this query keeps.
    pub fn count(&self) -> Handle<'d, u64> {
        self.book(Ok(What::Acc {
            label: "count".into(),
            exprs: Vec::new(),
            acc: Box::new(Count(0)),
        }))
    }

    /// The sum of `expr` over the entries this query keeps: of every object
    /// for one value per object, skipping missing values.
    pub fn sum(&self, expr: impl IntoExpr) -> Handle<'d, f64> {
        self.book(expr.into_expr().map(|e| What::Acc {
            label: format!("sum({e})"),
            exprs: vec![e],
            acc: Box::new(Moments::new(Stat::Sum)),
        }))
    }

    /// The mean of `expr`, as [`sum`](Self::sum) takes its values; `None`
    /// if there are none.
    pub fn mean(&self, expr: impl IntoExpr) -> Handle<'d, Option<f64>> {
        self.book_stat("mean", Stat::Mean, expr)
    }

    /// The smallest value of `expr`, as [`sum`](Self::sum) takes its
    /// values; `None` if there are none.
    pub fn min(&self, expr: impl IntoExpr) -> Handle<'d, Option<f64>> {
        self.book_stat("min", Stat::Min, expr)
    }

    /// The largest value of `expr`, as [`sum`](Self::sum) takes its
    /// values; `None` if there are none.
    pub fn max(&self, expr: impl IntoExpr) -> Handle<'d, Option<f64>> {
        self.book_stat("max", Stat::Max, expr)
    }

    /// `hist` filled at `x` (one expression per dimension) for every entry
    /// this query keeps, or for every object where an expression has one
    /// value per object; a missing value is not filled. `hist` keeps what
    /// it held.
    ///
    /// ```no_run
    /// # use oxiroot_dataset::Dataset;
    /// use oxihist::Hist;
    /// # let file = oxiroot_io_core::FileReader::open("nano.root")?;
    /// # let events = Dataset::open(&file, "Events")?;
    /// let muons = events.query().filter("len(Muon_pt) >= 2");
    /// let pt = muons.fill(Hist::reg(50, 0.0, 100.0).weight(), ["Muon_pt"]);
    /// let eta_phi = muons.fill(
    ///     Hist::reg(24, -2.4, 2.4).reg(32, -3.2, 3.2).weight(),
    ///     ["Muon_eta", "Muon_phi"],
    /// );
    /// let pt = pt.get()?; // one pass fills both
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    pub fn fill<H, E, const N: usize>(&self, hist: H, x: [E; N]) -> Handle<'d, H>
    where
        H: Fill<N>,
        E: IntoExpr,
    {
        self.book_fill(hist, x, None::<Expr>)
    }

    /// [`fill`](Self::fill), each point with the weight `weight`.
    pub fn fill_weighted<H, E, const N: usize>(
        &self,
        hist: H,
        x: [E; N],
        weight: impl IntoExpr,
    ) -> Handle<'d, H>
    where
        H: Fill<N>,
        E: IntoExpr,
    {
        self.book_fill(hist, x, Some(weight))
    }

    fn book_fill<H, E, W, const N: usize>(
        &self,
        hist: H,
        x: [E; N],
        weight: Option<W>,
    ) -> Handle<'d, H>
    where
        H: Fill<N>,
        E: IntoExpr,
        W: IntoExpr,
    {
        let exprs = x
            .into_iter()
            .map(IntoExpr::into_expr)
            .chain(weight.map(IntoExpr::into_expr))
            .collect::<std::result::Result<Vec<_>, _>>();
        self.book(exprs.map(|exprs| {
            let text: Vec<String> = exprs.iter().map(ToString::to_string).collect();
            What::Acc {
                label: format!("fill({})", text.join(", ")),
                acc: Box::new(Filled {
                    hist,
                    weighted: exprs.len() > N,
                }),
                exprs,
            }
        }))
    }

    /// What [`read`](Self::read) returns, the selected and defined columns
    /// of the entries this query keeps, in their order, but filled in the
    /// pass that fills the other actions, rather than a pass of its own.
    pub fn collect(&self) -> Handle<'d, Batch> {
        self.book(Ok(What::Collect))
    }

    /// The cut flow from the dataset to this query: for each filter on the
    /// way, how many entries reached it and how many it kept.
    pub fn report(&self) -> Handle<'d, Report> {
        self.book(Ok(What::Report))
    }

    /// Fill every action booked on the queries of this
    /// [`Dataset::query`] and not filled yet, in one pass. Their
    /// [`get`](Handle::get) then reads nothing.
    ///
    /// # Errors
    /// The first error of the pass; every action it was filling is then
    /// failed. An action that does not check fails alone, before the pass.
    ///
    /// [`Dataset::query`]: crate::Dataset::query
    pub fn run(&self) -> Result<()> {
        self.lock().run()
    }
}

/// An action that checked, ready to be filled.
struct Live {
    booked: Booked,
    /// For `collect`: the selected columns and the defined ones, typed.
    output: Option<(Vec<String>, Vec<(String, super::check::Ty)>)>,
    /// Each expression and the columns it reads.
    exprs: Vec<(Expr, Vec<(String, Source)>)>,
}

/// What every batch is taken through.
struct Pass<'g, 'd, 'f> {
    graph: &'g Graph<'d, 'f>,
    planned: Vec<Option<Planned>>,
    children: Vec<Vec<usize>>,
    actions_at: Vec<Vec<usize>>,
    live: Vec<Live>,
}

/// What one thread fills: a partial result per action (or why the action
/// failed: its values do not fit it), and per cut the entries that reached
/// it and those it kept.
struct Partial {
    accs: Vec<Option<Box<dyn Acc>>>,
    failed: Vec<Option<String>>,
    /// For `collect`: each batch's output, with the batch's index.
    collected: Vec<Vec<(usize, Batch)>>,
    counts: Vec<(u64, u64)>,
}

impl Pass<'_, '_, '_> {
    fn fresh(&self) -> Partial {
        Partial {
            accs: self
                .live
                .iter()
                .map(|l| match &l.booked.what {
                    What::Acc { acc, .. } => Some(acc.fresh()),
                    What::Report | What::Collect => None,
                })
                .collect(),
            failed: vec![None; self.live.len()],
            collected: vec![Vec::new(); self.live.len()],
            counts: vec![(0, 0); self.planned.len()],
        }
    }

    /// Take batch number `index` through the tree.
    fn take(&self, partial: &mut Partial, index: usize, batch: Batch) -> Result<()> {
        let state = State {
            batch,
            defined: Vec::new(),
        };
        self.visit(0, index, state, partial)
    }

    fn visit(&self, node: usize, index: usize, state: State, partial: &mut Partial) -> Result<()> {
        for &a in &self.actions_at[node] {
            let live = &self.live[a];
            if let Some((select, defined)) = &live.output
                && partial.failed[a].is_none()
            {
                match state.clone().into_output(select, defined) {
                    Ok(out) => partial.collected[a].push((index, out)),
                    Err(e) => partial.failed[a] = Some(format!("collect: {e}")),
                }
            }
            if let (Some(acc), What::Acc { label, .. }) = (&mut partial.accs[a], &live.booked.what)
            {
                // A value that does not fit fails this action alone.
                let added = live
                    .exprs
                    .iter()
                    .map(|(e, sources)| state.eval(e, sources))
                    .collect::<Result<Vec<_>>>()
                    .map_err(|e| e.to_string())
                    .and_then(|values| acc.add(&values, state.batch.len()));
                if let Err(m) = added {
                    partial.failed[a] = Some(format!("{label}: {m}"));
                    partial.accs[a] = None;
                }
            }
        }
        if let Some((&last, rest)) = self.children[node].split_last() {
            for &k in rest {
                self.step(k, index, state.clone(), partial)?;
            }
            self.step(last, index, state, partial)?;
        }
        Ok(())
    }

    fn step(
        &self,
        node: usize,
        index: usize,
        mut state: State,
        partial: &mut Partial,
    ) -> Result<()> {
        let planned = self.planned[node]
            .as_ref()
            .unwrap_or_else(|| unreachable!("every node of the pass is planned"));
        if let Some((all, passed)) = state.apply(&self.graph.nodes[node].step, planned)? {
            partial.counts[node].0 += all;
            partial.counts[node].1 += passed;
        }
        self.visit(node, index, state, partial)
    }
}

impl Partial {
    #[cfg(feature = "rayon")]
    fn merge(&mut self, other: Self) -> Result<()> {
        let pairs = self.accs.iter_mut().zip(other.accs);
        for ((a, b), (failed, other_failed)) in pairs.zip(self.failed.iter_mut().zip(other.failed))
        {
            if failed.is_none() && other_failed.is_some() {
                *failed = other_failed;
                *a = None;
            }
            if let (Some(acc), Some(b)) = (a.as_mut(), b)
                && let Err(m) = acc.merge(b)
            {
                *failed = Some(m);
                *a = None;
            }
        }
        for (a, b) in self.collected.iter_mut().zip(other.collected) {
            a.extend(b);
        }
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            a.0 += b.0;
            a.1 += b.1;
        }
        Ok(())
    }
}

impl Graph<'_, '_> {
    /// Fill every booked action in one pass.
    pub(super) fn run(&mut self) -> Result<()> {
        let booked = std::mem::take(&mut self.booked);
        let mut pass = Pass {
            graph: self,
            planned: (0..self.nodes.len()).map(|_| None).collect(),
            children: vec![Vec::new(); self.nodes.len()],
            actions_at: vec![Vec::new(); self.nodes.len()],
            live: Vec::new(),
        };
        let mut reads: Vec<String> = Vec::new();
        for b in booked {
            let plan = match self.plan_path(b.node) {
                Ok(p) => p,
                Err(e) => {
                    fill_slot(&b.slot, Outcome::Failed(e));
                    continue;
                }
            };
            let exprs = match &b.what {
                What::Acc { exprs, .. } => exprs
                    .iter()
                    .map(|e| {
                        check(e, &plan.columns())
                            .map(|c| (e.clone(), c.sources))
                            .map_err(|err| err.into_query_error(e))
                    })
                    .collect::<std::result::Result<Vec<_>, _>>(),
                What::Report | What::Collect => Ok(Vec::new()),
            };
            let exprs = match exprs {
                Ok(x) => x,
                Err(e) => {
                    fill_slot(&b.slot, Outcome::Failed(Error::other(e)));
                    continue;
                }
            };
            for (_, sources) in &exprs {
                for (_, s) in sources {
                    if let Source::Read { column, .. } = s
                        && !reads.contains(column)
                    {
                        reads.push(column.clone());
                    }
                }
            }
            for r in &plan.reads {
                if !reads.contains(r) {
                    reads.push(r.clone());
                }
            }
            let mut parent = 0;
            for (id, planned) in plan.steps {
                if pass.planned[id].is_none() {
                    pass.planned[id] = Some(planned);
                    pass.children[parent].push(id);
                }
                parent = id;
            }
            pass.actions_at[b.node].push(pass.live.len());
            let output = matches!(b.what, What::Collect)
                .then(|| (plan.select.clone(), plan.defined.clone()));
            pass.live.push(Live {
                booked: b,
                output,
                exprs,
            });
        }
        if pass.live.is_empty() {
            return Ok(());
        }
        match pass.fill(&reads) {
            Ok(partial) => {
                let results = partial
                    .accs
                    .into_iter()
                    .zip(partial.failed)
                    .zip(partial.collected);
                for (live, ((acc, failed), collected)) in pass.live.into_iter().zip(results) {
                    let outcome = match (live.booked.what, acc, failed) {
                        (_, _, Some(m)) => Outcome::Failed(Error::other(QueryError::new(m))),
                        (What::Collect, _, None) => match collect(collected) {
                            Ok(out) => Outcome::Done(Box::new(out)),
                            Err(e) => Outcome::Failed(e),
                        },
                        (What::Acc { acc: mut base, .. }, Some(filled), None) => {
                            match base.merge(filled) {
                                Ok(()) => Outcome::Done(base.finish()),
                                Err(m) => Outcome::Failed(Error::other(QueryError::new(m))),
                            }
                        }
                        _ => Outcome::Done(Box::new(Report {
                            cuts: self
                                .path(live.booked.node)
                                .into_iter()
                                .filter_map(|id| {
                                    self.nodes[id].step.cut_name().map(|name| Cut {
                                        name,
                                        all: partial.counts[id].0,
                                        passed: partial.counts[id].1,
                                    })
                                })
                                .collect(),
                        })),
                    };
                    fill_slot(&live.booked.slot, outcome);
                }
                Ok(())
            }
            Err(e) => {
                for live in &pass.live {
                    fill_slot(
                        &live.booked.slot,
                        Outcome::Failed(Error::other(QueryError::new(format!(
                            "the pass that fills this result failed: {e}"
                        )))),
                    );
                }
                Err(e)
            }
        }
    }
}

impl Pass<'_, '_, '_> {
    /// Every batch through the tree, on one thread or, with `rayon` and
    /// `parallel`, several.
    fn fill(&self, reads: &[String]) -> Result<Partial> {
        let batches = self.graph.batches(reads);
        #[cfg(feature = "rayon")]
        if self.graph.parallel
            && rayon::current_num_threads() > 1
            && rayon::current_thread_index().is_none()
        {
            return self.fill_parallel(batches);
        }
        let mut partial = self.fresh();
        for (index, batch) in batches.enumerate() {
            self.take(&mut partial, index, batch?)?;
        }
        Ok(partial)
    }

    /// This thread reads the batches (each read may itself run on the
    /// pool); the pool's threads take them through the tree, each into a
    /// partial result of its own, merged at the end. At most two batches
    /// per thread wait at once, so reading does not run ahead of the work.
    ///
    /// No lock is held while a batch is taken through the tree: a thread's
    /// partial result is taken out of its slot and put back after.
    #[cfg(feature = "rayon")]
    fn fill_parallel(&self, batches: impl Iterator<Item = Result<Batch>>) -> Result<Partial> {
        use std::sync::Condvar;
        let threads = rayon::current_num_threads();
        let slots: Vec<Mutex<Option<Partial>>> = (0..threads).map(|_| Mutex::new(None)).collect();
        let error: Mutex<Option<Error>> = Mutex::new(None);
        let waiting = (Mutex::new(0_usize), Condvar::new());
        let (slots, error, waiting) = (&slots, &error, &waiting);
        let fail = |e: Error| {
            error
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(e);
        };
        rayon::in_place_scope(|scope| {
            for (index, batch) in batches.enumerate() {
                if error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_some()
                {
                    break;
                }
                let batch = match batch {
                    Ok(b) => b,
                    Err(e) => {
                        fail(e);
                        break;
                    }
                };
                {
                    let mut n = waiting.0.lock().unwrap_or_else(PoisonError::into_inner);
                    while *n >= 2 * threads {
                        n = waiting.1.wait(n).unwrap_or_else(PoisonError::into_inner);
                    }
                    *n += 1;
                }
                scope.spawn(move |_| {
                    let slot = &slots[rayon::current_thread_index().unwrap_or(0) % threads];
                    let taken = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
                    let mut partial = taken.unwrap_or_else(|| self.fresh());
                    if let Err(e) = self.take(&mut partial, index, batch) {
                        fail(e);
                    }
                    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(other) = held.take()
                        && let Err(e) = partial.merge(other)
                    {
                        fail(e);
                    }
                    *held = Some(partial);
                    drop(held);
                    *waiting.0.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
                    waiting.1.notify_one();
                });
            }
        });
        if let Some(e) = error.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return Err(e);
        }
        let mut total = self.fresh();
        for slot in slots {
            if let Some(p) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                total.merge(p)?;
            }
        }
        Ok(total)
    }
}

/// Each batch's output, in the order the batches were read, as one batch.
fn collect(mut collected: Vec<(usize, Batch)>) -> Result<Batch> {
    collected.sort_by_key(|(i, _)| *i);
    let mut out: Option<Batch> = None;
    for (_, b) in collected {
        match &mut out {
            Some(o) => o.append(b)?,
            None => out = Some(b),
        }
    }
    Ok(out.unwrap_or_default())
}
