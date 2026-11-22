//! The function type: [`Func`], with [`Func1D`], [`Func2D`] and [`Func3D`] as
//! its one-, two- and three-dimensional forms, and their evaluation.

use oxifit_formula::{Formula, derivative, integrate};
use oxiroot_hist::GraphFunction;
use oxiroot_io_core::{Error, Result};
use std::ops::RangeInclusive;

/// A parametric function of `D` variables: a formula, its parameter values,
/// a range on each axis, and the fit results ROOT stores with it.
///
/// Use it through [`Func1D`], [`Func2D`] and [`Func3D`]; ROOT has no function
/// class past three dimensions, and none can be built here.
#[derive(Debug, Clone, PartialEq)]
pub struct Func<const D: usize> {
    /// What ROOT stores for the function: the name, the title (the formula as
    /// written), the formula in `[pN]` form, the parameters with their errors
    /// and limits, the `x` range and the fit's chi-square.
    record: GraphFunction,
    /// The record's formula, parsed.
    formula: Formula,
    /// The ranges of the axes after `x`; the record has `x`'s.
    outer: [(f64, f64); 2],
}

/// A 1-D parametric function `f(x; p)`, ROOT's `TF1`.
#[doc(alias = "TF1")]
pub type Func1D = Func<1>;
/// A 2-D parametric function `f(x, y; p)`, ROOT's `TF2`.
#[doc(alias = "TF2")]
pub type Func2D = Func<2>;
/// A 3-D parametric function `f(x, y, z; p)`, ROOT's `TF3`.
#[doc(alias = "TF3")]
pub type Func3D = Func<3>;

fn bounds(range: &RangeInclusive<f64>) -> (f64, f64) {
    (*range.start(), *range.end())
}

impl<const D: usize> Func<D> {
    /// A function from the formula `source`, unnamed, its parameters zero.
    fn build(source: &str, x: &RangeInclusive<f64>, outer: [(f64, f64); 2]) -> Result<Self> {
        let formula = Formula::parse(source)
            .map_err(|e| Error::InvalidInput(format!("bad formula {source:?}: {e}")))?;
        let npar = formula.npar();
        let (xmin, xmax) = bounds(x);
        Ok(Self {
            record: GraphFunction {
                name: String::new(),
                title: source.to_string(),
                formula: formula.root_formula().to_owned(),
                params: vec![0.0; npar],
                par_errors: vec![0.0; npar],
                par_min: vec![0.0; npar],
                par_max: vec![0.0; npar],
                xmin,
                xmax,
                chi2: 0.0,
                ndf: 0,
            },
            formula,
            outer,
        })
    }

    /// A function from the record ROOT stores and the ranges of its axes
    /// after `x`. The record's formula is kept in the form the parser writes.
    pub(crate) fn from_record(mut record: GraphFunction, outer: [(f64, f64); 2]) -> Result<Self> {
        let formula = Formula::parse(&record.formula).map_err(|e| {
            Error::Unsupported(format!("formula {:?} does not parse: {e}", record.formula))
        })?;
        formula.root_formula().clone_into(&mut record.formula);
        Ok(Self {
            record,
            formula,
            outer,
        })
    }

    /// The record ROOT stores: the `TF1` part of the function.
    pub(crate) const fn record(&self) -> &GraphFunction {
        &self.record
    }

    /// The range of axis `axis`, counted from 1 after `x`: 1 is `y`, 2 is `z`.
    pub(crate) const fn outer(&self, axis: usize) -> (f64, f64) {
        self.outer[axis - 1]
    }

    /// Set the key name this function is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.record.name = name.into();
        self
    }

    /// The key name (`fName`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.record.name
    }
    /// The formula expression as written (`fTitle`).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.record.title
    }
    /// The formula in ROOT's canonical `[pN]` form.
    #[must_use]
    pub fn formula(&self) -> &str {
        &self.record.formula
    }
    /// The number of parameters.
    #[must_use]
    pub fn npar(&self) -> usize {
        self.record.params.len()
    }
    /// The current parameter values.
    #[must_use]
    pub fn params(&self) -> &[f64] {
        &self.record.params
    }
    /// Parameter `i`, or `None` if the formula has no such parameter.
    #[must_use]
    pub fn param(&self, i: usize) -> Option<f64> {
        self.record.params.get(i).copied()
    }
    /// The parameter values, to change in place. Their number is fixed by
    /// the formula.
    pub fn params_mut(&mut self) -> &mut [f64] {
        &mut self.record.params
    }
    /// Set every parameter value (`[0], [1], …`), as a builder.
    ///
    /// # Errors
    /// [`Error::LengthMismatch`] if `params` does not hold one value per
    /// parameter of the formula.
    pub fn with_params(mut self, params: &[f64]) -> Result<Self> {
        oxiroot_io_core::raw::check_len(
            "function parameters",
            self.record.params.len(),
            params.len(),
        )?;
        self.record.params.copy_from_slice(params);
        Ok(self)
    }
    /// The fit χ² stored on the function (`fChisquare`), `0.0` if unfitted.
    #[must_use]
    pub fn chi2(&self) -> f64 {
        self.record.chi2
    }
    /// The fit degrees of freedom (`fNDF`).
    #[must_use]
    pub fn ndf(&self) -> i32 {
        self.record.ndf
    }
}

impl Func<1> {
    /// Build a `Func1D` named `name` from a ROOT `formula` over the range `x`,
    /// with all parameters initialised to zero.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(formula: &str, x: RangeInclusive<f64>) -> Result<Self> {
        Self::build(formula, &x, [(0.0, 0.0); 2])
    }

    /// Evaluate `f(x)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        self.formula.eval1(x, &self.record.params)
    }

    /// The definite integral `∫ₐᵇ f(x) dx` (adaptive Gauss–Kronrod), as
    /// `Func1D::Integral`.
    #[must_use]
    pub fn integral(&self, a: f64, b: f64) -> f64 {
        integrate(|x| self.eval(x), a, b)
    }

    /// The derivative `f'(x)` (Richardson central difference), as
    /// `Func1D::Derivative`.
    #[must_use]
    pub fn derivative(&self, x: f64) -> f64 {
        derivative(|x| self.eval(x), x)
    }

    /// The function's range (`fXmin..=fXmax`).
    #[must_use]
    pub const fn range(&self) -> RangeInclusive<f64> {
        self.record.xmin..=self.record.xmax
    }

    /// This function as the `Func1D` record a graph's `fFunctions` list holds, for
    /// example to attach it to a graph with
    /// [`Graph::with_function`](oxiroot_hist::Graph::with_function). The
    /// record's `formula` is the canonical `[pN]` form.
    ///
    /// ```
    /// use oxiroot_hist::Graph;
    /// use oxiroot_hist_func::Func1D;
    /// let f = Func1D::new("[0]+[1]*x", 0.0..=2.0)?.named("line").with_params(&[1.0, 2.0])?;
    /// let g = Graph::new(vec![0.0, 1.0], vec![1.0, 3.0])?.with_function(f.to_graph_function());
    /// assert_eq!(g.functions()[0].formula, "[p0]+[p1]*x");
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    #[must_use]
    pub fn to_graph_function(&self) -> GraphFunction {
        self.record.clone()
    }

    /// Rebuild a `Func1D` from a `Func1D` record, such as a function read from a
    /// graph's [`functions`](oxiroot_hist::Graph::functions), so it can be
    /// evaluated. The range is the record's `xmin`/`xmax`.
    ///
    /// # Errors
    /// Returns an error if the record's formula is not a valid expression.
    pub fn from_graph_function(function: GraphFunction) -> Result<Self> {
        Self::from_record(function, [(0.0, 0.0); 2])
    }
}

#[cfg(feature = "fit")]
impl Func<1> {
    /// Convert to a fittable [`Model`](oxifit::Model), seeded with this
    /// function's current parameters — so `data.fit(&tf1.to_model())` fits data
    /// to this function's shape. Requires the `fit` feature.
    ///
    /// The model evaluates the same formula as [`eval`](Func1D::eval). Its
    /// parameter names come from the title when the title is a formula with the
    /// same number of parameters (so `gaus` keeps its named parameters);
    /// otherwise, as for a read function whose title is free text, from the
    /// canonical `[pN]` formula.
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let formula = self.formula.clone();
        let names = model_names(self.title(), &formula);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        oxifit::Model::new(self.name(), &names, self.params().to_vec(), move |x, p| {
            formula.eval1(x, p)
        })
    }
}

/// A function's parameter names for a [`Model`](oxifit::Model): from its
/// title when the title is a formula with the same number of parameters (so
/// `gaus` keeps its named parameters), else from the canonical `[pN]` formula.
#[cfg(feature = "fit")]
fn model_names(title: &str, formula: &Formula) -> Vec<String> {
    match Formula::parse(title) {
        Ok(t) if t.npar() == formula.npar() => t.param_names().to_vec(),
        _ => formula.param_names().to_vec(),
    }
}

impl Func<2> {
    /// Build a `Func2D` from a formula in `x` and `y` over the ranges `x` and `y`.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(formula: &str, x: RangeInclusive<f64>, y: RangeInclusive<f64>) -> Result<Self> {
        Self::build(formula, &x, [bounds(&y), (0.0, 0.0)])
    }

    /// Evaluate `f(x, y)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        self.formula.eval(&[x, y], &self.record.params)
    }

    /// Convert to a fittable 2-D [`Model`](oxifit::Model), seeded with this
    /// function's current parameters — so `h2.fit(&tf2.to_model())` fits a 2-D
    /// histogram to this function's shape. Requires the `fit` feature;
    /// parameter names as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let formula = self.formula.clone();
        let names = model_names(self.title(), &formula);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        oxifit::Model::new2(
            self.name(),
            &names,
            self.params().to_vec(),
            move |x, y, p| formula.eval(&[x, y], p),
        )
    }

    /// The `x` and `y` ranges (`fXmin`/`fXmax`, `fYmin`/`fYmax`).
    #[must_use]
    pub const fn range(&self) -> (RangeInclusive<f64>, RangeInclusive<f64>) {
        let (ymin, ymax) = self.outer[0];
        (self.record.xmin..=self.record.xmax, ymin..=ymax)
    }
}

impl Func<3> {
    /// Build a `Func3D` from a formula in `x`, `y`, `z` over the given box.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(
        formula: &str,
        x: RangeInclusive<f64>,
        y: RangeInclusive<f64>,
        z: RangeInclusive<f64>,
    ) -> Result<Self> {
        Self::build(formula, &x, [bounds(&y), bounds(&z)])
    }

    /// Evaluate `f(x, y, z)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64, z: f64) -> f64 {
        self.formula.eval(&[x, y, z], &self.record.params)
    }

    /// Convert to a fittable 3-D [`Model`](oxifit::Model), seeded with this
    /// function's current parameters. Requires the `fit` feature; parameter
    /// names as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let formula = self.formula.clone();
        let names = model_names(self.title(), &formula);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        oxifit::Model::new3(
            self.name(),
            &names,
            self.params().to_vec(),
            move |x, y, z, p| formula.eval(&[x, y, z], p),
        )
    }

    /// The `x`, `y`, and `z` ranges.
    #[must_use]
    pub const fn range(
        &self,
    ) -> (
        RangeInclusive<f64>,
        RangeInclusive<f64>,
        RangeInclusive<f64>,
    ) {
        let [(ymin, ymax), (zmin, zmax)] = self.outer;
        (
            self.record.xmin..=self.record.xmax,
            ymin..=ymax,
            zmin..=zmax,
        )
    }
}
