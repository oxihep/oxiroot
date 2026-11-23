//! Queries over a [`Dataset`]: derived columns and cuts, checked against
//! the dataset's columns before anything is read, and evaluated a whole
//! column at a time.
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
//! # Records
//!
//! `Muon.pt` names the member `pt` of a record column `Muon` (an `RNTuple`
//! record, or a `std::vector` of structs), or, where there is none, the
//! column `Muon_pt`, as `NanoAOD` names its columns. A column whose own name
//! contains a dot is found by that name first.

mod check;
mod closure;
mod eval;
mod expr;
mod parse;

pub use check::QueryError;
pub use closure::{ArgColumn, ArgType, ArgValue, Callable, FromValue, IntoValue};
pub use expr::{Expr, col, lit, when};

use std::ops::Range;
use std::sync::Arc;

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

enum Step {
    Define(String, Expr),
    Filter(Expr),
    DefineWith(String, Vec<String>, Box<dyn Erased>),
    FilterWith(Vec<String>, Box<dyn Erased>),
}

/// A query over a [`Dataset`], built with [`Dataset::query`]: see the
/// [module documentation](self).
#[must_use = "a query reads nothing until `read` or `batches`"]
pub struct Query<'d, 'f> {
    dataset: &'d Dataset<'f>,
    select: Vec<String>,
    steps: Vec<Step>,
    error: Option<QueryError>,
    step: Option<u64>,
    range: Option<Range<u64>>,
    parallel: bool,
}

impl std::fmt::Debug for Query<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

/// One step per line, expressions in their text form.
impl std::fmt::Display for Query<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.select.is_empty() {
            writeln!(f, "select {}", self.select.join(", "))?;
        }
        for step in &self.steps {
            match step {
                Step::Define(n, e) => writeln!(f, "define {n} = {e}")?,
                Step::Filter(e) => writeln!(f, "filter {e}")?,
                Step::DefineWith(n, c, _) => {
                    writeln!(f, "define {n} = <closure>({})", c.join(", "))?
                }
                Step::FilterWith(c, _) => writeln!(f, "filter <closure>({})", c.join(", "))?,
            }
        }
        Ok(())
    }
}

impl<'f> Dataset<'f> {
    /// A query over this dataset: derived columns, cuts and selected
    /// columns, checked before anything is read. See [`Query`].
    pub fn query(&self) -> Query<'_, 'f> {
        Query {
            dataset: self,
            select: Vec::new(),
            steps: Vec::new(),
            error: None,
            step: None,
            range: None,
            parallel: false,
        }
    }
}

/// A checked step, with what it reads.
enum Planned {
    Define(String, Expr, Vec<(String, Source)>),
    Filter(Expr, Vec<(String, Source)>),
    DefineWith(String, Vec<Source>),
    FilterWith(Vec<Source>),
}

struct Plan {
    reads: Vec<String>,
    steps: Vec<Planned>,
    /// Every defined column and its type, in order: the type sets the
    /// output column's shape, the same in every batch.
    defined: Vec<(String, Ty)>,
}

impl<'d, 'f> Query<'d, 'f> {
    fn push(mut self, step: std::result::Result<Step, QueryError>) -> Self {
        match step {
            Ok(s) => self.steps.push(s),
            Err(e) => {
                self.error.get_or_insert(e);
            }
        }
        self
    }

    /// Keep these dataset columns in the output, as they are (any type,
    /// strings included), filtered like everything else. Defined columns
    /// are always in the output, after these.
    pub fn select<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.select
            .extend(columns.into_iter().map(|s| s.as_ref().to_owned()));
        self
    }

    /// Add the column `name`, computed from `expr`. Later steps can use it.
    pub fn define(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        let name = name.into();
        let step = expr.into_expr().map(|e| Step::Define(name, e));
        self.push(step)
    }

    /// Keep the entries where `expr`, one bool per entry, is true.
    pub fn filter(self, expr: impl IntoExpr) -> Self {
        let step = expr.into_expr().map(Step::Filter);
        self.push(step)
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
    pub fn define_with<Args, R, F, I, S>(self, name: impl Into<String>, columns: I, f: F) -> Self
    where
        Args: 'static,
        R: IntoValue,
        F: Callable<Args, R>,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns = columns.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.push(Ok(Step::DefineWith(
            name.into(),
            columns,
            Box::new(Closure::<F, Args, R>::new(f)),
        )))
    }

    /// Keep the entries where `f`, given `columns` entry by entry, returns
    /// `true` (and not where an argument is missing). Argument types as for
    /// [`define_with`](Self::define_with).
    pub fn filter_with<Args, F, I, S>(self, columns: I, f: F) -> Self
    where
        Args: 'static,
        F: Callable<Args, bool>,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns = columns.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.push(Ok(Step::FilterWith(
            columns,
            Box::new(Closure::<F, Args, bool>::new(f)),
        )))
    }

    /// Read in batches of about `entries` entries (as [`Batches::step`]).
    ///
    /// [`Batches::step`]: crate::Batches::step
    pub fn step(mut self, entries: u64) -> Self {
        self.step = Some(entries);
        self
    }

    /// Only entries `range` of the dataset.
    pub fn range(mut self, range: Range<u64>) -> Self {
        self.range = Some(range);
        self
    }

    /// Read each batch's columns concurrently (as [`Batches::parallel`]).
    ///
    /// [`Batches::parallel`]: crate::Batches::parallel
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
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
        self.plan().map(|_| ())
    }

    fn plan(&self) -> Result<Plan> {
        if let Some(e) = &self.error {
            return Err(Error::other(e.clone()));
        }
        let schema = self.dataset.schema()?;
        let dataset: Vec<(String, DataType)> = schema
            .iter()
            .map(|(n, t)| (n.to_owned(), t.clone()))
            .collect();
        let mut reads: Vec<String> = Vec::new();
        for name in &self.select {
            if !dataset.iter().any(|(n, _)| n == name) {
                let q = QueryError::new(format!("no column `{name}` to select"));
                return Err(Error::other(q));
            }
            if !reads.contains(name) {
                reads.push(name.clone());
            }
        }
        let mut defined: Vec<(String, Ty)> = Vec::new();
        let mut steps = Vec::new();
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
        for step in &self.steps {
            let columns = Columns {
                dataset: &dataset,
                defined: &defined,
            };
            match step {
                Step::Define(name, expr) => {
                    let c = check(expr, &columns)
                        .map_err(|e| Error::other(e.into_query_error(expr)))?;
                    c.sources.iter().for_each(|(_, s)| read_of(s, &mut reads));
                    fresh(name, &defined)?;
                    steps.push(Planned::Define(name.clone(), expr.clone(), c.sources));
                    defined.push((name.clone(), c.ty));
                }
                Step::Filter(expr) => {
                    let c = check(expr, &columns)
                        .map_err(|e| Error::other(e.into_query_error(expr)))?;
                    c.sources.iter().for_each(|(_, s)| read_of(s, &mut reads));
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
                    steps.push(Planned::Filter(expr.clone(), c.sources));
                }
                Step::DefineWith(name, cols, f) => {
                    let sources = self.closure_columns(cols, f.as_ref(), &columns)?;
                    sources.iter().for_each(|s| read_of(s, &mut reads));
                    fresh(name, &defined)?;
                    defined.push((name.clone(), f.output()));
                    steps.push(Planned::DefineWith(name.clone(), sources));
                }
                Step::FilterWith(cols, f) => {
                    let sources = self.closure_columns(cols, f.as_ref(), &columns)?;
                    sources.iter().for_each(|s| read_of(s, &mut reads));
                    let out = f.output();
                    if out.scalar != Scalar::Bool || out.objects.is_some() {
                        return Err(Error::other(QueryError::new(
                            "a filter closure returns one bool per entry",
                        )));
                    }
                    steps.push(Planned::FilterWith(sources));
                }
            }
        }
        // A query that reads no column still needs the entries: read the
        // first column for its count (it is not in the output).
        if reads.is_empty()
            && let Some((first, _)) = dataset.first()
        {
            reads.push(first.clone());
        }
        Ok(Plan {
            reads,
            steps,
            defined,
        })
    }

    fn closure_columns(
        &self,
        cols: &[String],
        f: &dyn Erased,
        columns: &Columns<'_>,
    ) -> Result<Vec<Source>> {
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

    /// Run the query, every batch, into one batch: the selected columns, then
    /// the defined ones, for the entries the filters keep.
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
        let plan = Arc::new(self.plan()?);
        let mut batches = self.dataset.batches(&plan.reads).parallel(self.parallel);
        if let Some(step) = self.step {
            batches = batches.step(step);
        }
        if let Some(range) = &self.range {
            batches = batches.range(range.clone());
        }
        Ok(batches.map(move |b| b.and_then(|b| self.run(&plan, b))))
    }

    /// Apply the plan's steps to one batch.
    fn run(&self, plan: &Plan, mut batch: Batch) -> Result<Batch> {
        let mut defined: Vec<(String, Val)> = Vec::new();
        for (step, planned) in self.steps.iter().zip(&plan.steps) {
            let entries = batch.len();
            let value_of = |source: &Source| -> std::result::Result<Val, String> {
                match source {
                    Source::Defined(n) => defined
                        .iter()
                        .find(|(d, _)| d == n)
                        .map(|(_, v)| v.clone())
                        .ok_or_else(|| format!("no column `{n}`")),
                    Source::Read { column, member } => {
                        let c = batch
                            .get(column)
                            .ok_or_else(|| format!("no column `{column}`"))?;
                        match member {
                            Some(m) => Val::from_member(column, c, m),
                            None => Val::from_column(column, c),
                        }
                    }
                }
            };
            match planned {
                Planned::Define(_, expr, sources) | Planned::Filter(expr, sources) => {
                    let lookup = |name: &str| -> std::result::Result<Val, String> {
                        let source = sources
                            .iter()
                            .find(|(n, _)| n == name)
                            .map(|(_, s)| s)
                            .ok_or_else(|| format!("no column `{name}`"))?;
                        value_of(source)
                    };
                    let value = Evaluator::new(entries, &lookup)
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
                        })?;
                    match planned {
                        Planned::Define(name, ..) => defined.push((name.clone(), value)),
                        _ => {
                            let mask = mask_of(&value);
                            batch = batch.filter(&mask)?;
                            for (_, v) in &mut defined {
                                *v = v.filter(&mask);
                            }
                        }
                    }
                }
                Planned::DefineWith(_, sources) | Planned::FilterWith(sources) => {
                    let f = match step {
                        Step::DefineWith(_, _, f) | Step::FilterWith(_, f) => f,
                        _ => unreachable!("a closure step plans to a closure step"),
                    };
                    let args = sources
                        .iter()
                        .map(&value_of)
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|m| Error::other(QueryError::new(m)))?;
                    let value = f.run(&args, entries).map_err(|(i, m)| {
                        let column = match step {
                            Step::DefineWith(_, c, _) | Step::FilterWith(c, _) => c[i].clone(),
                            _ => String::new(),
                        };
                        Error::other(QueryError::new(format!(
                            "argument {} of a closure, from column `{column}`: {m}",
                            i + 1
                        )))
                    })?;
                    match planned {
                        Planned::DefineWith(name, ..) => defined.push((name.clone(), value)),
                        _ => {
                            let mask = mask_of(&value);
                            batch = batch.filter(&mask)?;
                            for (_, v) in &mut defined {
                                *v = v.filter(&mask);
                            }
                        }
                    }
                }
            }
        }
        let mut out = batch.select(&self.select)?;
        for ((name, value), (_, ty)) in defined.into_iter().zip(&plan.defined) {
            out.insert(name, value.into_column(ty.nullable))?;
        }
        Ok(out)
    }
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
