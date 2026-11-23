//! Queries over a [`Dataset`]: derived columns and cuts, checked against
//! the dataset's columns before anything is read, and evaluated a whole
//! column at a time; and actions (histograms, sums, counts, a cut flow)
//! booked on them and filled together in one pass.
//!
//! A query is built with [`Dataset::query`]. Its steps run in order:
//! [`select`](Query::select) names dataset columns to keep as they are,
//! [`define`](Query::define) adds a column computed from an [`Expr`],
//! [`filter`](Query::filter) keeps the entries an expression says, and
//! [`define_with`](Query::define_with) / [`filter_with`](Query::filter_with)
//! do the same with a typed Rust closure, for what an expression cannot say.
//!
//! An expression is built in Rust ([`col`], [`lit`], [`when`] and the
//! [`Expr`] methods) or written as text ([`Expr::parse`], or a `&str`
//! passed to `define`/`filter`). Both are the same tree: an expression
//! prints as its text, and the text parses back to it.
//!
//! ```no_run
//! use oxiroot_dataset::Dataset;
//! use oxiroot_dataset::query::col;
//! # let file = oxiroot_io_core::FileReader::open("nano.root")?;
//!
//! let events = Dataset::open(&file, "Events")?;
//! let out = events
//!     .query()
//!     .select(["run", "event"])
//!     .filter(col("nMuon").ge(2))
//!     .filter("any(Muon.pt > 20 and abs(Muon.eta) < 2.4)")
//!     .define("lead_pt", col("Muon_pt").max())
//!     .define_with("ht", ["Jet_pt"], |pts: Vec<f32>| pts.iter().sum::<f32>())
//!     .read()?;
//! # Ok::<(), oxiroot_io_core::Error>(())
//! ```
//!
//! # Values per entry and per object
//!
//! A column holds one value per entry (`nMuon`) or a list of them, one per
//! object (`Muon_pt`). An operation between the two applies to each object:
//! `Muon_pt * weight` scales every muon of an entry by its weight. Two
//! per-object columns combine object by object, and must have the same
//! number of objects in each entry. [`any`](Expr::any), [`all`](Expr::all),
//! [`count`](Expr::count), [`sum`](Expr::sum), [`max`](Expr::max),
//! [`min`](Expr::min), [`mean`](Expr::mean), [`len`](Expr::len) and
//! [`at`](Expr::at) bring objects back to one value per entry. A filter
//! takes one bool per entry: `Muon_pt > 20` (one per muon) is refused, with
//! a hint to say `any(…)` or `all(…)`.
//!
//! # Missing values
//!
//! [`at`](Expr::at) past the last object, [`max`](Expr::max) of no objects,
//! or a value of an optional column, is missing. A missing value stays
//! missing through arithmetic and comparisons, and a filter does not keep an
//! entry where its condition is missing (`false and missing` is still false,
//! `true or missing` true). [`unwrap_or`](Expr::unwrap_or) (`??` in text)
//! gives a default. A defined column that can be missing is an optional
//! column.
//!
//! # Actions and branches
//!
//! An action books a result on a query and returns a [`Handle`]:
//! [`count`](Query::count), [`sum`](Query::sum), [`mean`](Query::mean),
//! [`min`](Query::min), [`max`](Query::max), [`fill`](Query::fill) and
//! [`fill_weighted`](Query::fill_weighted) (a [`Fill`] histogram, one
//! expression per dimension), [`report`](Query::report) (the cut flow,
//! with the names [`filter_named`](Query::filter_named) gives) and
//! [`collect`](Query::collect) (what [`read`](Query::read) returns).
//! Booking reads nothing; the first [`Handle::get`] runs one pass over the
//! dataset that fills every action booked on queries of the same
//! [`Dataset::query`].
//!
//! A step makes a new query and leaves the one it is called on as it was,
//! so a query can be the base of several branches. They share its steps,
//! computed once per batch, and their actions fill in the same pass:
//!
//! ```no_run
//! use oxihist::Hist;
//! # use oxiroot_dataset::Dataset;
//! # let file = oxiroot_io_core::FileReader::open("nano.root")?;
//! # let events = Dataset::open(&file, "Events")?;
//! let base = events
//!     .query()
//!     .filter_named("two muons", "len(Muon_pt) == 2")
//!     .define("ht", "sum(Jet_pt)");
//! let central = base.filter("all(abs(Muon_eta) < 1.2)");
//! let forward = base.filter("any(abs(Muon_eta) >= 1.2)");
//! let pt = central.fill(Hist::reg(50, 0.0, 100.0).weight(), ["Muon_pt"]);
//! let (n_central, n_forward) = (central.count(), forward.count());
//! let ht = forward.sum("ht");
//! let flow = central.report();
//!
//! let pt = pt.get()?; // one pass fills all five
//! println!("{} central, {} forward\n{}", n_central.get()?, n_forward.get()?, flow.get()?);
//! # Ok::<(), oxiroot_io_core::Error>(())
//! ```
//!
//! # Records
//!
//! `Muon.pt` names the member `pt` of a record column `Muon` (an `RNTuple`
//! record, or a `std::vector` of structs), or, where there is none, the
//! column `Muon_pt`, as `NanoAOD` names its columns. A column whose own name
//! contains a dot is found by that name first.

mod action;
mod check;
mod closure;
mod eval;
mod expr;
mod parse;

pub use action::{Cut, Fill, Handle, Report};
pub use check::QueryError;
pub use closure::{ArgColumn, ArgType, ArgValue, Callable, FromValue, IntoValue};
pub use expr::{Expr, col, lit, when};

use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oxiroot_io_core::{Error, Result};

use crate::{Batch, DataType, Dataset};
use check::{Columns, Scalar, Source, Ty, check};
use closure::{Closure, Erased};
use eval::{Evaluator, Val};

/// What a query step can be given as an expression: an [`Expr`], or its
/// text (`&str`, `String`), parsed when the query is checked.
pub trait IntoExpr {
    /// The expression.
    ///
    /// # Errors
    /// Text that does not parse.
    fn into_expr(self) -> std::result::Result<Expr, QueryError>;
}

impl IntoExpr for Expr {
    fn into_expr(self) -> std::result::Result<Expr, QueryError> {
        Ok(self)
    }
}
impl IntoExpr for &str {
    fn into_expr(self) -> std::result::Result<Expr, QueryError> {
        Expr::parse(self)
    }
}
impl IntoExpr for String {
    fn into_expr(self) -> std::result::Result<Expr, QueryError> {
        Expr::parse(&self)
    }
}
impl IntoExpr for &String {
    fn into_expr(self) -> std::result::Result<Expr, QueryError> {
        Expr::parse(self)
    }
}

/// What a node of the query adds to the one it is built on.
enum Step {
    /// The dataset itself: node 0.
    Root,
    Select(Vec<String>),
    Define(String, Expr),
    /// A cut, with the name it has in a [`Report`].
    Filter(Option<String>, Expr),
    DefineWith(String, Vec<String>, Box<dyn Erased>),
    FilterWith(Option<String>, Vec<String>, Box<dyn Erased>),
    /// A step that could not be built (text that does not parse): every
    /// query built on it fails its check with this.
    Invalid(QueryError),
}

impl Step {
    /// The name a cut has in a report: its own, or its expression's text.
    fn cut_name(&self) -> Option<String> {
        match self {
            Self::Filter(name, e) => Some(name.clone().unwrap_or_else(|| e.to_string())),
            Self::FilterWith(name, c, _) => Some(
                name.clone()
                    .unwrap_or_else(|| format!("<closure>({})", c.join(", "))),
            ),
            _ => None,
        }
    }
}

struct Node {
    parent: usize,
    step: Step,
}

/// Every query built from one [`Dataset::query`], as a tree of steps, and
/// the actions booked on it, waiting for the pass that fills them.
struct Graph<'d, 'f> {
    dataset: &'d Dataset<'f>,
    nodes: Vec<Node>,
    step: Option<u64>,
    range: Option<Range<u64>>,
    parallel: bool,
    booked: Vec<action::Booked>,
}

/// A query over a [`Dataset`], built with [`Dataset::query`]: see the
/// [module documentation](self).
///
/// A query is a handle on one node of a tree of steps. Each step
/// (`filter`, `define`, …) makes a new query on top of this one and leaves
/// this one as it was, so one query can be the base of several: they share
/// its steps, and their actions are filled in the same pass.
#[must_use = "a query reads nothing until an action's `get`, `run`, `read` or `batches`"]
pub struct Query<'d, 'f> {
    graph: Arc<Mutex<Graph<'d, 'f>>>,
    node: usize,
}

impl Clone for Query<'_, '_> {
    fn clone(&self) -> Self {
        Self {
            graph: Arc::clone(&self.graph),
            node: self.node,
        }
    }
}

impl std::fmt::Debug for Query<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

/// One step per line, from the dataset to this query, expressions in their
/// text form.
impl std::fmt::Display for Query<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.lock();
        for id in g.path(self.node) {
            match &g.nodes[id].step {
                Step::Root => {}
                Step::Select(c) => writeln!(f, "select {}", c.join(", "))?,
                Step::Define(n, e) => writeln!(f, "define {n} = {e}")?,
                Step::Filter(Some(n), e) => writeln!(f, "filter {e}  # {n}")?,
                Step::Filter(None, e) => writeln!(f, "filter {e}")?,
                Step::DefineWith(n, c, _) => {
                    writeln!(f, "define {n} = <closure>({})", c.join(", "))?;
                }
                Step::FilterWith(n, c, _) => {
                    write!(f, "filter <closure>({})", c.join(", "))?;
                    match n {
                        Some(n) => writeln!(f, "  # {n}")?,
                        None => writeln!(f)?,
                    }
                }
                Step::Invalid(e) => writeln!(f, "<invalid: {}>", e.message())?,
            }
        }
        Ok(())
    }
}

impl<'f> Dataset<'f> {
    /// A query over this dataset: derived columns, cuts and selected
    /// columns, checked before anything is read, and actions (histograms,
    /// sums, counts, a cut flow) filled in one pass. See [`Query`].
    pub fn query(&self) -> Query<'_, 'f> {
        Query {
            graph: Arc::new(Mutex::new(Graph {
                dataset: self,
                nodes: vec![Node {
                    parent: 0,
                    step: Step::Root,
                }],
                step: None,
                range: None,
                parallel: false,
                booked: Vec::new(),
            })),
            node: 0,
        }
    }
}

/// A checked step, with what it reads.
enum Planned {
    Root,
    Select,
    Define(String, Vec<(String, Source)>),
    Filter(Vec<(String, Source)>),
    DefineWith(String, Vec<Source>),
    FilterWith(Vec<Source>),
}

/// The steps from the dataset to one query, checked.
struct PathPlan {
    /// Each node on the path after the root, and its checked step.
    steps: Vec<(usize, Planned)>,
    select: Vec<String>,
    reads: Vec<String>,
    /// Every defined column and its type, in order: the type sets the
    /// output column's shape, the same in every batch.
    defined: Vec<(String, Ty)>,
    /// The dataset's columns, for checking an action at this query.
    dataset: Vec<(String, DataType)>,
}

impl PathPlan {
    fn columns(&self) -> Columns<'_> {
        Columns {
            dataset: &self.dataset,
            defined: &self.defined,
        }
    }
}

/// What a batch has become at one node: its rows the filters kept, and the
/// columns defined so far.
#[derive(Clone)]
struct State {
    batch: Batch,
    defined: Vec<(String, Val)>,
}

impl State {
    fn value_of(&self, source: &Source) -> std::result::Result<Val, String> {
        match source {
            Source::Defined(n) => self
                .defined
                .iter()
                .find(|(d, _)| d == n)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| format!("no column `{n}`")),
            Source::Read { column, member } => {
                let c = self
                    .batch
                    .get(column)
                    .ok_or_else(|| format!("no column `{column}`"))?;
                match member {
                    Some(m) => Val::from_member(column, c, m),
                    None => Val::from_column(column, c),
                }
            }
        }
    }

    /// `expr` over this state's entries.
    fn eval(&self, expr: &Expr, sources: &[(String, Source)]) -> Result<Val> {
        let lookup = |name: &str| -> std::result::Result<Val, String> {
            let source = sources
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, s)| s)
                .ok_or_else(|| format!("no column `{name}`"))?;
            self.value_of(source)
        };
        Evaluator::new(self.batch.len(), &lookup)
            .eval(&expr.0)
            .map_err(|e| {
                Error::other(
                    check::NodeError {
                        id: e.id,
                        message: e.message,
                        hint: None,
                    }
                    .into_query_error(expr),
                )
            })
    }

    /// The output columns: the selected ones, then the defined ones, each
    /// shaped by its type.
    fn into_output(self, select: &[String], defined: &[(String, Ty)]) -> Result<Batch> {
        let mut out = self.batch.select(select)?;
        for ((name, value), (_, ty)) in self.defined.into_iter().zip(defined) {
            out.insert(name, value.into_column(ty.nullable))?;
        }
        Ok(out)
    }

    fn keep(&mut self, value: &Val) -> Result<()> {
        let mask = mask_of(value);
        self.batch = self.batch.filter(&mask)?;
        for (_, v) in &mut self.defined {
            *v = v.filter(&mask);
        }
        Ok(())
    }

    /// Apply one step. For a cut: how many entries reached it, and how many
    /// it kept.
    fn apply(&mut self, step: &Step, planned: &Planned) -> Result<Option<(u64, u64)>> {
        let before = self.batch.len() as u64;
        match (step, planned) {
            (Step::Define(_, expr), Planned::Define(name, sources)) => {
                let value = self.eval(expr, sources)?;
                self.defined.push((name.clone(), value));
            }
            (Step::Filter(_, expr), Planned::Filter(sources)) => {
                let value = self.eval(expr, sources)?;
                self.keep(&value)?;
                return Ok(Some((before, self.batch.len() as u64)));
            }
            (
                Step::DefineWith(_, columns, f) | Step::FilterWith(_, columns, f),
                Planned::DefineWith(_, sources) | Planned::FilterWith(sources),
            ) => {
                let args = sources
                    .iter()
                    .map(|s| self.value_of(s))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|m| Error::other(QueryError::new(m)))?;
                let value = f.run(&args, self.batch.len()).map_err(|(i, m)| {
                    Error::other(QueryError::new(format!(
                        "argument {} of a closure, from column `{}`: {m}",
                        i + 1,
                        columns[i]
                    )))
                })?;
                if let Planned::DefineWith(name, _) = planned {
                    self.defined.push((name.clone(), value));
                } else {
                    self.keep(&value)?;
                    return Ok(Some((before, self.batch.len() as u64)));
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

impl<'d, 'f> Graph<'d, 'f> {
    /// The nodes from the root (excluded) to `node`, in order.
    fn path(&self, mut node: usize) -> Vec<usize> {
        let mut path = Vec::new();
        while node != 0 {
            path.push(node);
            node = self.nodes[node].parent;
        }
        path.reverse();
        path
    }

    fn plan_path(&self, node: usize) -> Result<PathPlan> {
        let schema = self.dataset.schema()?;
        let dataset: Vec<(String, DataType)> = schema
            .iter()
            .map(|(n, t)| (n.to_owned(), t.clone()))
            .collect();
        let mut plan = PathPlan {
            steps: Vec::new(),
            select: Vec::new(),
            reads: Vec::new(),
            defined: Vec::new(),
            dataset: Vec::new(),
        };
        let read_of = |s: &Source, reads: &mut Vec<String>| {
            if let Source::Read { column, .. } = s
                && !reads.contains(column)
            {
                reads.push(column.clone());
            }
        };
        // A defined column may not hide another: later steps would read it
        // in place of the dataset's.
        let fresh = |name: &str, defined: &[(String, Ty)]| -> Result<()> {
            if defined.iter().any(|(n, _)| n == name) || dataset.iter().any(|(n, _)| n == name) {
                return Err(Error::other(
                    QueryError::new(format!("the query already has a column `{name}`"))
                        .with_hint(Some("give the defined column another name".into())),
                ));
            }
            Ok(())
        };
        for id in self.path(node) {
            let columns = Columns {
                dataset: &dataset,
                defined: &plan.defined,
            };
            let planned = match &self.nodes[id].step {
                Step::Root => Planned::Root,
                Step::Invalid(e) => return Err(Error::other(e.clone())),
                Step::Select(names) => {
                    for name in names {
                        if !dataset.iter().any(|(n, _)| n == name) {
                            let q = QueryError::new(format!("no column `{name}` to select"));
                            return Err(Error::other(q));
                        }
                        if !plan.select.contains(name) {
                            plan.select.push(name.clone());
                        }
                        if !plan.reads.contains(name) {
                            plan.reads.push(name.clone());
                        }
                    }
                    Planned::Select
                }
                Step::Define(name, expr) => {
                    let c = check(expr, &columns)
                        .map_err(|e| Error::other(e.into_query_error(expr)))?;
                    c.sources
                        .iter()
                        .for_each(|(_, s)| read_of(s, &mut plan.reads));
                    fresh(name, &plan.defined)?;
                    plan.defined.push((name.clone(), c.ty));
                    Planned::Define(name.clone(), c.sources)
                }
                Step::Filter(_, expr) => {
                    let c = check(expr, &columns)
                        .map_err(|e| Error::other(e.into_query_error(expr)))?;
                    c.sources
                        .iter()
                        .for_each(|(_, s)| read_of(s, &mut plan.reads));
                    if c.ty.scalar != Scalar::Bool || c.ty.objects.is_some() {
                        let hint = (c.ty.scalar == Scalar::Bool)
                            .then(|| "say which: any(…) or all(…)".to_owned());
                        let what = match &c.ty.objects {
                            Some(o) => format!("one {} per object of `{o}`", c.ty.scalar.name()),
                            None => format!("one {} per entry", c.ty.scalar.name()),
                        };
                        let text = expr.to_string();
                        let span = 0..text.len();
                        return Err(Error::other(
                            QueryError::new(format!(
                                "a filter needs one bool per entry, but this is {what}"
                            ))
                            .at(text, span)
                            .with_hint(hint),
                        ));
                    }
                    Planned::Filter(c.sources)
                }
                Step::DefineWith(name, cols, f) => {
                    let sources = closure_columns(cols, f.as_ref(), &columns)?;
                    sources.iter().for_each(|s| read_of(s, &mut plan.reads));
                    fresh(name, &plan.defined)?;
                    plan.defined.push((name.clone(), f.output()));
                    Planned::DefineWith(name.clone(), sources)
                }
                Step::FilterWith(_, cols, f) => {
                    let sources = closure_columns(cols, f.as_ref(), &columns)?;
                    sources.iter().for_each(|s| read_of(s, &mut plan.reads));
                    let out = f.output();
                    if out.scalar != Scalar::Bool || out.objects.is_some() {
                        return Err(Error::other(QueryError::new(
                            "a filter closure returns one bool per entry",
                        )));
                    }
                    Planned::FilterWith(sources)
                }
            };
            plan.steps.push((id, planned));
        }
        // A query that reads no column still needs the entries: read the
        // first column for its count (it is not in the output).
        if plan.reads.is_empty()
            && let Some((first, _)) = dataset.first()
        {
            plan.reads.push(first.clone());
        }
        plan.dataset = dataset;
        Ok(plan)
    }

    /// The batches of the whole query, reading `reads`.
    fn batches(&self, reads: &[String]) -> crate::Batches<'d, 'f> {
        let mut batches = self.dataset.batches(reads).parallel(self.parallel);
        if let Some(step) = self.step {
            batches = batches.step(step);
        }
        if let Some(range) = &self.range {
            batches = batches.range(range.clone());
        }
        batches
    }
}

impl<'d, 'f> Query<'d, 'f> {
    fn lock(&self) -> MutexGuard<'_, Graph<'d, 'f>> {
        self.graph.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new query: this one, then `step`.
    fn then(&self, step: Step) -> Self {
        let mut g = self.lock();
        g.nodes.push(Node {
            parent: self.node,
            step,
        });
        let node = g.nodes.len() - 1;
        drop(g);
        Self {
            graph: Arc::clone(&self.graph),
            node,
        }
    }

    fn then_expr(&self, expr: impl IntoExpr, step: impl FnOnce(Expr) -> Step) -> Self {
        self.then(expr.into_expr().map_or_else(Step::Invalid, step))
    }

    /// Keep these dataset columns in the output of [`read`](Self::read) and
    /// [`batches`](Self::batches), as they are (any type, strings
    /// included), filtered like everything else. Defined columns are always
    /// in the output, after these.
    pub fn select<I, S>(&self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns = columns.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.then(Step::Select(columns))
    }

    /// Add the column `name`, computed from `expr`. Later steps can use it.
    pub fn define(&self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        let name = name.into();
        self.then_expr(expr, |e| Step::Define(name, e))
    }

    /// Keep the entries where `expr`, one bool per entry, is true. In a
    /// [`report`](Self::report) the cut is named by its expression.
    pub fn filter(&self, expr: impl IntoExpr) -> Self {
        self.then_expr(expr, |e| Step::Filter(None, e))
    }

    /// [`filter`](Self::filter), with the name the cut has in a
    /// [`report`](Self::report).
    pub fn filter_named(&self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        let name = name.into();
        self.then_expr(expr, |e| Step::Filter(Some(name), e))
    }

    /// Add the column `name`, computed by `f` from `columns`, entry by
    /// entry. Name the argument types in the closure: a number or `bool`
    /// per entry, an `Option` for a column with missing values, a `Vec` for
    /// one value per object. They are checked against the columns before
    /// anything is read.
    ///
    /// ```no_run
    /// # use oxiroot_dataset::Dataset;
    /// # let file = oxiroot_io_core::FileReader::open("nano.root")?;
    /// # let events = Dataset::open(&file, "Events")?;
    /// let out = events
    ///     .query()
    ///     .define_with("mt", ["pt", "met", "dphi"], |pt: f32, met: f32, dphi: f32| {
    ///         (2.0 * pt * met * (1.0 - dphi.cos())).sqrt()
    ///     })
    ///     .read()?;
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    ///
    /// A value that does not fit its argument's type (an integer out of
    /// range) is an error, naming the argument, the column and the value.
    pub fn define_with<Args, R, F, I, S>(&self, name: impl Into<String>, columns: I, f: F) -> Self
    where
        Args: 'static,
        R: IntoValue,
        F: Callable<Args, R>,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns = columns.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.then(Step::DefineWith(
            name.into(),
            columns,
            Box::new(Closure::<F, Args, R>::new(f)),
        ))
    }

    /// Keep the entries where `f`, given `columns` entry by entry, returns
    /// `true` (and not where an argument is missing). Argument types as for
    /// [`define_with`](Self::define_with).
    pub fn filter_with<Args, F, I, S>(&self, columns: I, f: F) -> Self
    where
        Args: 'static,
        F: Callable<Args, bool>,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns = columns.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.then(Step::FilterWith(
            None,
            columns,
            Box::new(Closure::<F, Args, bool>::new(f)),
        ))
    }

    /// Read in batches of about `entries` entries (as [`Batches::step`]).
    /// Like [`range`](Self::range) and [`parallel`](Self::parallel), this
    /// holds for every query built from the same [`Dataset::query`].
    ///
    /// [`Batches::step`]: crate::Batches::step
    pub fn step(&self, entries: u64) -> Self {
        self.lock().step = Some(entries);
        self.clone()
    }

    /// Only entries `range` of the dataset, for every query built from the
    /// same [`Dataset::query`].
    pub fn range(&self, range: Range<u64>) -> Self {
        self.lock().range = Some(range);
        self.clone()
    }

    /// Read each batch's columns concurrently (as [`Batches::parallel`]),
    /// and, with the `rayon` feature, fill actions from several batches at
    /// once, merging what each thread filled. For every query built from
    /// the same [`Dataset::query`].
    ///
    /// [`Batches::parallel`]: crate::Batches::parallel
    pub fn parallel(&self, parallel: bool) -> Self {
        self.lock().parallel = parallel;
        self.clone()
    }

    /// Check the query against the dataset's columns without reading any
    /// data: every column exists, every expression has the types its
    /// operations need, every filter is one bool per entry, every closure's
    /// arguments fit their columns.
    ///
    /// # Errors
    /// A [`QueryError`] (inside [`Error::Other`]) saying what is wrong and
    /// where, or a read error of the dataset's schema.
    pub fn check(&self) -> Result<()> {
        self.lock().plan_path(self.node).map(|_| ())
    }

    /// Run the query, every batch, into one batch: the selected columns, then
    /// the defined ones, for the entries the filters keep. This is a pass
    /// of its own; booked actions are not filled by it.
    ///
    /// # Errors
    /// A [`QueryError`] (inside [`Error::Other`]) if the query does not
    /// check, or a value does not fit (two per-object columns with
    /// different numbers of objects); a read error.
    pub fn read(&self) -> Result<Batch> {
        let mut out: Option<Batch> = None;
        for batch in self.batches()? {
            let batch = batch?;
            match &mut out {
                Some(o) => o.append(batch)?,
                None => out = Some(batch),
            }
        }
        Ok(out.unwrap_or_default())
    }

    /// Run the query batch by batch, as the dataset is read: for a dataset
    /// too large to hold. The query is checked first, so a mistake is an
    /// error here, before anything is read.
    ///
    /// # Errors
    /// As [`read`](Self::read), for the check.
    pub fn batches(&self) -> Result<impl Iterator<Item = Result<Batch>> + '_> {
        let g = self.lock();
        let plan = Arc::new(g.plan_path(self.node)?);
        let batches = g.batches(&plan.reads);
        drop(g);
        Ok(batches.map(move |b| b.and_then(|b| self.output(&plan, b))))
    }

    /// One batch through the path's steps, as the output columns.
    fn output(&self, plan: &PathPlan, batch: Batch) -> Result<Batch> {
        let g = self.lock();
        let mut state = State {
            batch,
            defined: Vec::new(),
        };
        for (id, planned) in &plan.steps {
            state.apply(&g.nodes[*id].step, planned)?;
        }
        state.into_output(&plan.select, &plan.defined)
    }
}

/// The sources of a closure's columns, checked against its arguments.
fn closure_columns(cols: &[String], f: &dyn Erased, columns: &Columns<'_>) -> Result<Vec<Source>> {
    if cols.len() != f.arity() {
        return Err(Error::other(QueryError::new(format!(
            "a closure of {} argument{} is given {} column{}: {}",
            f.arity(),
            if f.arity() == 1 { "" } else { "s" },
            cols.len(),
            if cols.len() == 1 { "" } else { "s" },
            cols.join(", ")
        ))));
    }
    let mut sources = Vec::new();
    let mut tys = Vec::new();
    for c in cols {
        let (s, t) = columns
            .resolve(c)
            .map_err(|(m, h)| Error::other(QueryError::new(m).with_hint(h)))?;
        sources.push(s);
        tys.push(t);
    }
    if let Some(i) = f.rejects(&tys) {
        let t = &tys[i];
        let what = match (&t.objects, t.nullable) {
            (Some(_), _) => format!("one {} per object: take it as a `Vec`", t.scalar.name()),
            (None, true) => format!(
                "a {} that can be missing: take it as an `Option`",
                t.scalar.name()
            ),
            (None, false) => format!("one {} per entry", t.scalar.name()),
        };
        return Err(Error::other(QueryError::new(format!(
            "argument {} of the closure ({}) does not fit column `{}`, {what}",
            i + 1,
            f.signature(),
            cols[i]
        ))));
    }
    Ok(sources)
}

/// The entries a filter's value keeps: true, and present.
fn mask_of(v: &Val) -> Vec<bool> {
    match &v.data {
        eval::Data::Bool(b) => b
            .iter()
            .enumerate()
            .map(|(i, &x)| x && v.valid.as_ref().is_none_or(|valid| valid[i]))
            .collect(),
        _ => Vec::new(),
    }
}
