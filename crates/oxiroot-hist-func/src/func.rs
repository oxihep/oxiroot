//! The function types: [`Func1D`], [`Func2D`] and [`Func3D`], their evaluation, and the
//! shared [`FuncCore`] (name, formula, parameters, fit metadata).

use oxifit_formula::{Formula, derivative, integrate};
use oxiroot_hist::GraphFunction;
use oxiroot_io_core::{Error, Result};
use std::ops::RangeInclusive;

/// The data shared by [`Func1D`]/[`Func2D`]/[`Func3D`]: a name and title, the parsed
/// formula, the parameter values, and the fit-result metadata ROOT stores
/// (`fParErrors`/`fParMin`/`fParMax`/`fChisquare`/`fNDF`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FuncCore {
    name: String,
    /// ROOT's `fTitle` — the formula as written by the user (`[0]` form).
    title: String,
    formula: Formula,
    params: Vec<f64>,
    par_errors: Vec<f64>,
    par_min: Vec<f64>,
    par_max: Vec<f64>,
    chi2: f64,
    ndf: i32,
}

impl FuncCore {
    fn build(name: &str, source: &str) -> Result<Self> {
        let formula = Formula::parse(source)
            .map_err(|e| Error::InvalidInput(format!("bad formula {source:?}: {e}")))?;
        let npar = formula.npar();
        Ok(Self {
            name: name.to_string(),
            title: source.to_string(),
            params: vec![0.0; npar],
            par_errors: vec![0.0; npar],
            par_min: vec![0.0; npar],
            par_max: vec![0.0; npar],
            chi2: 0.0,
            ndf: 0,
            formula,
        })
    }

    /// The `Func1D` record ROOT stores for this function over `[xmin, xmax]`.
    pub(crate) fn record(&self, xmin: f64, xmax: f64) -> GraphFunction {
        GraphFunction {
            name: self.name.clone(),
            title: self.title.clone(),
            formula: self.formula.root_formula().to_owned(),
            params: self.params.clone(),
            par_errors: self.par_errors.clone(),
            par_min: self.par_min.clone(),
            par_max: self.par_max.clone(),
            xmin,
            xmax,
            chi2: self.chi2,
            ndf: self.ndf,
        }
    }

    /// Parse a `Func1D` record's formula back into a function core.
    pub(crate) fn from_record(f: GraphFunction) -> Result<Self> {
        let formula = Formula::parse(&f.formula).map_err(|e| {
            Error::Unsupported(format!("formula {:?} does not parse: {e}", f.formula))
        })?;
        Ok(Self {
            name: f.name,
            title: f.title,
            formula,
            params: f.params,
            par_errors: f.par_errors,
            par_min: f.par_min,
            par_max: f.par_max,
            chi2: f.chi2,
            ndf: f.ndf,
        })
    }
}

/// The accessors and parameter mutators common to `Func1D`/`Func2D`/`Func3D`.
macro_rules! accessors {
    () => {
        /// The key name (`fName`).
        #[must_use]
        pub fn name(&self) -> &str {
            &self.core.name
        }
        /// The formula expression as written (`fTitle`).
        #[must_use]
        pub fn title(&self) -> &str {
            &self.core.title
        }
        /// The formula in ROOT's canonical `[pN]` form.
        #[must_use]
        pub fn formula(&self) -> &str {
            self.core.formula.root_formula()
        }
        /// The number of parameters.
        #[must_use]
        pub fn npar(&self) -> usize {
            self.core.params.len()
        }
        /// The current parameter values.
        #[must_use]
        pub fn params(&self) -> &[f64] {
            &self.core.params
        }
        /// Parameter `i`, or `None` if the formula has no such parameter.
        #[must_use]
        pub fn param(&self, i: usize) -> Option<f64> {
            self.core.params.get(i).copied()
        }
        /// The parameter values, to change in place. Their number is fixed by
        /// the formula.
        pub fn params_mut(&mut self) -> &mut [f64] {
            &mut self.core.params
        }
        /// Set every parameter value (`[0], [1], …`), as a builder.
        ///
        /// # Errors
        /// [`Error::LengthMismatch`] if `params` does not hold one value per
        /// parameter of the formula.
        pub fn with_params(mut self, params: &[f64]) -> Result<Self> {
            oxiroot_io_core::raw::check_len(
                "function parameters",
                self.core.params.len(),
                params.len(),
            )?;
            self.core.params.copy_from_slice(params);
            Ok(self)
        }
        /// The fit χ² stored on the function (`fChisquare`), `0.0` if unfitted.
        #[must_use]
        pub fn chi2(&self) -> f64 {
            self.core.chi2
        }
        /// The fit degrees of freedom (`fNDF`).
        #[must_use]
        pub fn ndf(&self) -> i32 {
            self.core.ndf
        }
    };
}

/// A `Func1D` — a 1-D parametric function `f(x; p)`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TF1")]
pub struct Func1D {
    pub(crate) core: FuncCore,
    pub(crate) xmin: f64,
    pub(crate) xmax: f64,
}

impl Func1D {
    /// Build a `Func1D` named `name` from a ROOT `formula` over the range `x`,
    /// with all parameters initialised to zero.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(name: &str, formula: &str, x: RangeInclusive<f64>) -> Result<Self> {
        Ok(Self {
            core: FuncCore::build(name, formula)?,
            xmin: *x.start(),
            xmax: *x.end(),
        })
    }

    /// Set the key name this function is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.core.name = name.into();
        self
    }

    /// Evaluate `f(x)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        self.core.formula.eval1(x, &self.core.params)
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

    accessors!();

    /// The function's range (`fXmin..=fXmax`).
    #[must_use]
    pub const fn range(&self) -> RangeInclusive<f64> {
        self.xmin..=self.xmax
    }

    /// This function as the `Func1D` record a graph's `fFunctions` list holds, for
    /// example to attach it to a graph with
    /// [`Graph::with_function`](oxiroot_hist::Graph::with_function). The
    /// record's `formula` is the canonical `[pN]` form.
    ///
    /// ```
    /// use oxiroot_hist::Graph;
    /// use oxiroot_hist_func::Func1D;
    /// let f = Func1D::new("line", "[0]+[1]*x", 0.0..=2.0)?.with_params(&[1.0, 2.0])?;
    /// let g = Graph::new(vec![0.0, 1.0], vec![1.0, 3.0])?.with_function(f.to_graph_function());
    /// assert_eq!(g.functions()[0].formula, "[p0]+[p1]*x");
    /// # Ok::<(), oxiroot_io_core::Error>(())
    /// ```
    #[must_use]
    pub fn to_graph_function(&self) -> GraphFunction {
        self.core.record(self.xmin, self.xmax)
    }

    /// Rebuild a `Func1D` from a `Func1D` record, such as a function read from a
    /// graph's [`functions`](oxiroot_hist::Graph::functions), so it can be
    /// evaluated. The range is the record's `xmin`/`xmax`.
    ///
    /// # Errors
    /// Returns an error if the record's formula is not a valid expression.
    pub fn from_graph_function(function: GraphFunction) -> Result<Self> {
        let (xmin, xmax) = (function.xmin, function.xmax);
        Ok(Self {
            core: FuncCore::from_record(function)?,
            xmin,
            xmax,
        })
    }
}

#[cfg(feature = "fit")]
impl Func1D {
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
        let formula = self.core.formula.clone();
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

/// A `Func2D` — a 2-D parametric function `f(x, y; p)`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TF2")]
pub struct Func2D {
    pub(crate) core: FuncCore,
    pub(crate) xmin: f64,
    pub(crate) xmax: f64,
    pub(crate) ymin: f64,
    pub(crate) ymax: f64,
}

impl Func2D {
    /// Build a `Func2D` from a formula in `x` and `y` over the ranges `x` and `y`.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(
        name: &str,
        formula: &str,
        x: RangeInclusive<f64>,
        y: RangeInclusive<f64>,
    ) -> Result<Self> {
        Ok(Self {
            core: FuncCore::build(name, formula)?,
            xmin: *x.start(),
            xmax: *x.end(),
            ymin: *y.start(),
            ymax: *y.end(),
        })
    }

    /// Set the key name this function is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.core.name = name.into();
        self
    }

    /// Evaluate `f(x, y)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        self.core.formula.eval(&[x, y], &self.core.params)
    }

    /// Convert to a fittable 2-D [`Model`](oxifit::Model), seeded with this
    /// function's current parameters — so `h2.fit(&tf2.to_model())` fits a 2-D
    /// histogram to this function's shape. Requires the `fit` feature;
    /// parameter names as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let formula = self.core.formula.clone();
        let names = model_names(self.title(), &formula);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        oxifit::Model::new2(
            self.name(),
            &names,
            self.params().to_vec(),
            move |x, y, p| formula.eval(&[x, y], p),
        )
    }

    accessors!();

    /// The `x` and `y` ranges (`fXmin`/`fXmax`, `fYmin`/`fYmax`).
    #[must_use]
    pub const fn range(&self) -> (RangeInclusive<f64>, RangeInclusive<f64>) {
        (self.xmin..=self.xmax, self.ymin..=self.ymax)
    }
}

/// A `Func3D` — a 3-D parametric function `f(x, y, z; p)`.
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TF3")]
pub struct Func3D {
    pub(crate) core: FuncCore,
    pub(crate) xmin: f64,
    pub(crate) xmax: f64,
    pub(crate) ymin: f64,
    pub(crate) ymax: f64,
    pub(crate) zmin: f64,
    pub(crate) zmax: f64,
}

impl Func3D {
    /// Build a `Func3D` from a formula in `x`, `y`, `z` over the given box.
    ///
    /// # Errors
    /// Returns an error if `formula` is not a valid expression.
    pub fn new(
        name: &str,
        formula: &str,
        x: RangeInclusive<f64>,
        y: RangeInclusive<f64>,
        z: RangeInclusive<f64>,
    ) -> Result<Self> {
        Ok(Self {
            core: FuncCore::build(name, formula)?,
            xmin: *x.start(),
            xmax: *x.end(),
            ymin: *y.start(),
            ymax: *y.end(),
            zmin: *z.start(),
            zmax: *z.end(),
        })
    }

    /// Set the key name this function is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.core.name = name.into();
        self
    }

    /// Evaluate `f(x, y, z)` with the current parameters.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64, z: f64) -> f64 {
        self.core.formula.eval(&[x, y, z], &self.core.params)
    }

    /// Convert to a fittable 3-D [`Model`](oxifit::Model), seeded with this
    /// function's current parameters. Requires the `fit` feature; parameter
    /// names as for [`Func1D::to_model`].
    #[cfg(feature = "fit")]
    #[must_use]
    pub fn to_model(&self) -> oxifit::Model {
        let formula = self.core.formula.clone();
        let names = model_names(self.title(), &formula);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        oxifit::Model::new3(
            self.name(),
            &names,
            self.params().to_vec(),
            move |x, y, z, p| formula.eval(&[x, y, z], p),
        )
    }

    accessors!();

    /// The `x`, `y`, and `z` ranges.
    #[must_use]
    pub const fn range(
        &self,
    ) -> (
        RangeInclusive<f64>,
        RangeInclusive<f64>,
        RangeInclusive<f64>,
    ) {
        (
            self.xmin..=self.xmax,
            self.ymin..=self.ymax,
            self.zmin..=self.zmax,
        )
    }
}
