//! The function type: [`Func`], with [`Func1D`], [`Func2D`] and [`Func3D`] as
//! its one-, two- and three-dimensional forms, and their evaluation.

use oxiformula::{Formula, derivative, integrate};
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
    /// written), the formula as ROOT compiles it and its parameters' names,
    /// the parameters with their errors and limits, the `x` range and the
    /// fit's chi-square.
    record: GraphFunction,
    /// The record's formula, parsed.
    formula: Formula,
    /// Its derivative in `x`, where it has a closed form.
    dx: Option<Formula>,
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
        // The parse error stays whole (`Error::other`): through `?`, the
        // `oxiroot` facade makes it its `Error::Formula`.
        let formula = Formula::parse(source).map_err(Error::other)?;
        if let Some(var) = beyond::<D>(&formula) {
            return Err(Error::InvalidInput(format!(
                "formula {source:?} uses `{var}`, but a {D}-dimensional function has only {}",
                ["x", "x and y", "x, y and z"][D - 1]
            )));
        }
        let npar = formula.npar();
        let (xmin, xmax) = bounds(x);
        Ok(Self {
            record: GraphFunction {
                name: String::new(),
                title: source.to_string(),
                formula: formula.to_root_named(),
                param_names: formula.param_names().to_vec(),
                params: vec![0.0; npar],
                par_errors: vec![0.0; npar],
                par_min: vec![0.0; npar],
                par_max: vec![0.0; npar],
                xmin,
                xmax,
                chi2: 0.0,
                ndf: 0,
            },
            dx: formula.variable_derivative(0),
            formula,
            outer,
        })
    }

    /// A function from the record ROOT stores and the ranges of its axes
    /// after `x`. The record's formula is kept as this crate writes it, with
    /// the names the record gives its parameters.
    pub(crate) fn from_record(mut record: GraphFunction, outer: [(f64, f64); 2]) -> Result<Self> {
        let mut formula = Formula::parse(&record.formula).map_err(|e| {
            Error::Unsupported(format!("formula {:?} does not parse: {e}", record.formula))
        })?;
        if formula.npar() != record.params.len() {
            return Err(Error::Unsupported(format!(
                "formula {:?} has {} parameters, but the function stores {}",
                record.formula,
                formula.npar(),
                record.params.len()
            )));
        }
        if let Some(var) = beyond::<D>(&formula) {
            return Err(Error::Unsupported(format!(
                "formula {:?} uses `{var}`, more variables than a {D}-dimensional function has",
                record.formula
            )));
        }
        // The file's names, when they fit (a file of another writer may have
        // `p0`… for a formula of named parameters, or none).
        if let Ok(named) = formula.clone().with_param_names(&record.param_names) {
            formula = named;
        }
        record.formula = formula.to_root_named();
        record.param_names = formula.param_names().to_vec();
        Ok(Self {
            record,
            dx: formula.variable_derivative(0),
            formula,
            outer,
        })
    }

    /// ROOT's shortcut `name` alone, a 1-D function: built in, so it parses.
    fn shortcut(name: &str, x: &RangeInclusive<f64>) -> Self {
        match Self::build(name, x, [(0.0, 0.0); 2]) {
            Ok(f) => f,
            Err(e) => unreachable!("ROOT's `{name}` parses: {e}"),
        }
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
    /// The formula as a file stores it (`TFormula::fFormula`): ROOT's
    /// spelling, with each parameter by its name (`[p0]`, `[Mean]`); the
    /// names' order is [`param_names`](Self::param_names).
    #[must_use]
    pub fn formula(&self) -> &str {
        &self.record.formula
    }
    /// The parameter names, by index: `p0`, `p1`, … for numbered
    /// parameters, and names as the formula gives them, such as `Constant`,
    /// `Mean`, `Sigma` for `gaus`. A file stores them with the function.
    #[must_use]
    pub fn param_names(&self) -> &[String] {
        &self.record.param_names
    }
    /// The value of the parameter named `name` (`"Mean"`, or `"p1"` and
    /// `"1"` for parameter 1), or `None` if there is none.
    #[must_use]
    pub fn param_named(&self, name: &str) -> Option<f64> {
        self.formula.param_index(name).and_then(|i| self.param(i))
    }
    /// Set the parameter named `name`, as a builder.
    ///
    /// ```
    /// use oxiroot_hist_func::Func1D;
    /// let f = Func1D::new("gaus", -5.0..=5.0)?
    ///     .with_param("Constant", 10.0)?
    ///     .with_param("Sigma", 2.0)?;
    /// assert_eq!(f.params(), [10.0, 0.0, 2.0]);
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if no parameter has this name.
    pub fn with_param(mut self, name: &str, value: f64) -> Result<Self> {
        self.set_param(name, value)?;
        Ok(self)
    }
    /// Set the parameter named `name`.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if no parameter has this name.
    pub fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let i = self.formula.param_index(name).ok_or_else(|| {
            Error::InvalidInput(format!(
                "no parameter named {name:?}; the parameters are {}",
                self.record.param_names.join(", ")
            ))
        })?;
        self.record.params[i] = value;
        Ok(())
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

    /// ROOT's `gaus` over `x`: `Constant·exp(−½((x − Mean)/Sigma)²)`, its
    /// parameters `Constant`, `Mean`, `Sigma`, all zero.
    ///
    /// ```
    /// use oxiroot_hist_func::Func1D;
    /// let g = Func1D::gaus(-5.0..=5.0).with_params(&[2.0, 0.0, 1.0])?;
    /// assert_eq!(g.param_names(), ["Constant", "Mean", "Sigma"]);
    /// assert_eq!(g.eval(0.0), 2.0);
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    #[must_use]
    pub fn gaus(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("gaus", &x)
    }
    /// ROOT's `gausn`: the Gaussian of area `Constant`.
    #[must_use]
    pub fn gausn(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("gausn", &x)
    }
    /// ROOT's `expo`: `exp(Constant + Slope·x)`.
    #[must_use]
    pub fn expo(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("expo", &x)
    }
    /// ROOT's `landau`: `Constant·TMath::Landau(x, MPV, Sigma)`.
    #[must_use]
    pub fn landau(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("landau", &x)
    }
    /// ROOT's `crystalball`: parameters `Constant`, `Mean`, `Sigma`, `Alpha`,
    /// `N`.
    #[must_use]
    pub fn crystalball(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("crystalball", &x)
    }
    /// ROOT's `breitwigner`: `p0` times the unit-area Breit–Wigner of mean
    /// `p1` and width `p2`.
    #[must_use]
    pub fn breitwigner(x: RangeInclusive<f64>) -> Self {
        Self::shortcut("breitwigner", &x)
    }
    /// ROOT's `pol<degree>`: `p0 + p1·x + … + pN·xᴺ`.
    ///
    /// # Errors
    /// [`Error::Other`] if the polynomial has more parameters than a formula
    /// may (10 000).
    pub fn pol(degree: usize, x: RangeInclusive<f64>) -> Result<Self> {
        Self::new(&format!("pol{degree}"), x)
    }
    /// ROOT's `cheb<degree>`: `Σ pₖ·Tₖ(x)`, Chebyshev polynomials up to
    /// degree 10.
    ///
    /// # Errors
    /// [`Error::Other`] for a degree past 10.
    pub fn cheb(degree: usize, x: RangeInclusive<f64>) -> Result<Self> {
        Self::new(&format!("cheb{degree}"), x)
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
        let f = self.formula.compile();
        integrate(|x| f.eval(&[x], &self.record.params), a, b)
    }

    /// The derivative `f'(x)`: exact, from the formula's symbolic
    /// derivative; for a formula with no closed form here (`TMath::Landau`,
    /// the special densities), a Richardson central difference, as ROOT's
    /// `TF1::Derivative` takes.
    #[must_use]
    pub fn derivative(&self, x: f64) -> f64 {
        match &self.dx {
            Some(d) => d.eval1(x, &self.record.params),
            None => derivative(|x| self.eval(x), x),
        }
    }

    /// The function's range (`fXmin..=fXmax`).
    #[must_use]
    pub const fn range(&self) -> RangeInclusive<f64> {
        self.record.xmin..=self.record.xmax
    }

    /// This function as the `Func1D` record a graph's `fFunctions` list holds, for
    /// example to attach it to a graph with
    /// [`Graph::with_function`](oxiroot_hist::Graph::with_function). The
    /// record's `formula` names its parameters, as ROOT stores it, with their
    /// order beside it.
    ///
    /// ```
    /// use oxiroot_hist::Graph;
    /// use oxiroot_hist_func::Func1D;
    /// let f = Func1D::new("[a]+[b]*x", 0.0..=2.0)?.named("line").with_params(&[1.0, 2.0])?;
    /// let g = Graph::new(vec![0.0, 1.0], vec![1.0, 3.0])?.with_function(f.to_graph_function());
    /// assert_eq!(g.functions()[0].formula, "[a]+[b]*x");
    /// assert_eq!(g.functions()[0].param_names, ["a", "b"]);
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
    /// The model evaluates the same formula as [`eval`](Func1D::eval),
    /// compiled, with its exact gradient where the formula has one; its
    /// parameter names are the function's ([`param_names`](Self::param_names)).
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let value = self.formula.compile();
        let model = oxifit::Model::new(
            self.name(),
            &self.name_refs(),
            self.params().to_vec(),
            move |x, p| value.eval(&[x], p),
        );
        self.with_gradient(model)
    }
}

#[cfg(feature = "fit")]
impl<const D: usize> Func<D> {
    fn name_refs(&self) -> Vec<&str> {
        self.record.param_names.iter().map(String::as_str).collect()
    }

    /// `model` with the formula's symbolic gradient, compiled, when every
    /// parameter has one.
    fn with_gradient(&self, model: oxifit::Model) -> oxifit::Model {
        let derivatives: Option<Vec<oxiformula::Compiled>> = (0..self.formula.npar())
            .map(|k| self.formula.derivative(k).map(|d| d.compile()))
            .collect();
        match derivatives {
            Some(d) => model.with_gradient_at(move |c, p, g| {
                for (gk, dk) in g.iter_mut().zip(&d) {
                    *gk = dk.eval(c, p);
                }
            }),
            None => model,
        }
    }
}

/// The first variable `formula` uses that a function of `D` variables lacks.
fn beyond<const D: usize>(formula: &Formula) -> Option<&'static str> {
    formula.variables().into_iter().find(|v| {
        let index = ["x", "y", "z", "t"]
            .iter()
            .position(|n| n == v)
            .unwrap_or(0);
        index >= D
    })
}

impl Func<2> {
    /// Build a `Func2D` from a formula in `x` and `y` over the ranges `x` and `y`.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(formula: &str, x: RangeInclusive<f64>, y: RangeInclusive<f64>) -> Result<Self> {
        Self::build(formula, &x, [bounds(&y), (0.0, 0.0)])
    }

    /// ROOT's `xygaus`: a Gaussian in `x` times one in `y`, parameters
    /// `Constant`, `MeanX`, `SigmaX`, `MeanY`, `SigmaY`.
    #[must_use]
    pub fn gaus(x: RangeInclusive<f64>, y: RangeInclusive<f64>) -> Self {
        Self::shortcut2("xygaus", &x, &y)
    }
    /// ROOT's `bigaus`: the correlated 2-D Gaussian, parameters `Constant`,
    /// `MeanX`, `SigmaX`, `MeanY`, `SigmaY`, `Rho`.
    #[must_use]
    pub fn bigaus(x: RangeInclusive<f64>, y: RangeInclusive<f64>) -> Self {
        Self::shortcut2("bigaus", &x, &y)
    }

    fn shortcut2(name: &str, x: &RangeInclusive<f64>, y: &RangeInclusive<f64>) -> Self {
        match Self::build(name, x, [bounds(y), (0.0, 0.0)]) {
            Ok(f) => f,
            Err(e) => unreachable!("ROOT's `{name}` parses: {e}"),
        }
    }

    /// Evaluate `f(x, y)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        self.formula.eval(&[x, y], &self.record.params)
    }

    /// Convert to a fittable 2-D [`Model`](oxifit::Model), seeded with this
    /// function's current parameters — so `h2.fit(&tf2.to_model())` fits a 2-D
    /// histogram to this function's shape. Requires the `fit` feature;
    /// parameter names and gradient as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let value = self.formula.compile();
        let model = oxifit::Model::new2(
            self.name(),
            &self.name_refs(),
            self.params().to_vec(),
            move |x, y, p| value.eval(&[x, y], p),
        );
        self.with_gradient(model)
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
    /// names and gradient as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let value = self.formula.compile();
        let model = oxifit::Model::new3(
            self.name(),
            &self.name_refs(),
            self.params().to_vec(),
            move |x, y, z, p| value.eval(&[x, y, z], p),
        );
        self.with_gradient(model)
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
