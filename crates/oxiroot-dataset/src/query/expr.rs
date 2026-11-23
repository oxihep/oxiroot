//! The expression tree of a query, the Rust builder over it, and its text
//! form (`Display`).

use std::fmt::{self, Write};
use std::ops;

/// A literal value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Lit {
    Bool(bool),
    Int(i64),
    Float(f64),
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Neg,
    Not,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// An element-wise function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Func {
    Sqrt,
    Abs,
    Exp,
    Log,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Floor,
    Ceil,
    Round,
    Atan2,
    Hypot,
    Min,
    Max,
}

/// A reduction from the objects of an entry to one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reduce {
    Any,
    All,
    Count,
    Sum,
    Max,
    Min,
    Mean,
    Len,
}

/// A node of the tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Col(String),
    Lit(Lit),
    Unary(UnOp, Box<Self>),
    Binary(BinOp, Box<Self>, Box<Self>),
    Call(Func, Vec<Self>),
    Reduce(Reduce, Box<Self>),
    /// The object at an index (negative from the end), missing if there is
    /// none.
    At(Box<Self>, i64),
    /// The objects of the first that the second (a per-object bool) keeps.
    Keep(Box<Self>, Box<Self>),
    /// The first, or the second where the first is missing.
    UnwrapOr(Box<Self>, Box<Self>),
    /// `if c then a else b`.
    If(Box<Self>, Box<Self>, Box<Self>),
}

/// An expression over a dataset's columns, for [`Query::define`] and
/// [`Query::filter`]: built in Rust from [`col`] and [`lit`], or parsed from
/// its text form ([`Expr::parse`]), which is what it prints as.
///
/// Arithmetic uses `+ - * /` and [`pow`](Self::pow); comparisons are methods
/// ([`gt`](Self::gt), [`eq`](Self::eq), …), and logic uses `&`, `|` and `!`.
/// An operation between a column of one value per entry and one of one
/// value per object applies to each object; [`any`](Self::any),
/// [`count`](Self::count), [`sum`](Self::sum), [`at`](Self::at), … bring
/// objects back to one value per entry.
///
/// ```
/// use oxiroot_dataset::query::{col, lit};
///
/// let good = col("Muon_pt").gt(20.0) & col("Muon_eta").abs().lt(2.4);
/// let e = col("nMuon").ge(2) & good.any();
/// assert_eq!(
///     e.to_string(),
///     "nMuon >= 2 and any(Muon_pt > 20.0 and abs(Muon_eta) < 2.4)"
/// );
/// ```
///
/// [`Query::define`]: crate::Query::define
/// [`Query::filter`]: crate::Query::filter
#[derive(Debug, Clone, PartialEq)]
pub struct Expr(pub(crate) Node);

/// The column `name`: a column of the dataset (`Muon_pt`), a member of a
/// record (`Muon.pt`, which also finds `Muon_pt`), or a column defined
/// earlier in the query.
#[must_use]
pub fn col(name: impl Into<String>) -> Expr {
    Expr(Node::Col(name.into()))
}

/// A literal: a `bool`, an integer, or a float.
#[must_use]
pub fn lit(value: impl Into<Expr>) -> Expr {
    value.into()
}

/// `then` where `condition` holds, `otherwise` where it does not, and
/// missing where `condition` is.
#[must_use]
pub fn when(condition: impl Into<Expr>, then: impl Into<Expr>, otherwise: impl Into<Expr>) -> Expr {
    Expr(Node::If(
        Box::new(condition.into().0),
        Box::new(then.into().0),
        Box::new(otherwise.into().0),
    ))
}

impl From<bool> for Expr {
    fn from(v: bool) -> Self {
        Self(Node::Lit(Lit::Bool(v)))
    }
}
impl From<i64> for Expr {
    fn from(v: i64) -> Self {
        Self(Node::Lit(Lit::Int(v)))
    }
}
impl From<i32> for Expr {
    fn from(v: i32) -> Self {
        Self(Node::Lit(Lit::Int(i64::from(v))))
    }
}
impl From<u32> for Expr {
    fn from(v: u32) -> Self {
        Self(Node::Lit(Lit::Int(i64::from(v))))
    }
}
impl From<f64> for Expr {
    fn from(v: f64) -> Self {
        Self(Node::Lit(Lit::Float(v)))
    }
}
impl From<f32> for Expr {
    fn from(v: f32) -> Self {
        Self(Node::Lit(Lit::Float(f64::from(v))))
    }
}

macro_rules! unary_fns {
    ($($(#[$m:meta])* $name:ident => $f:ident),* $(,)?) => {$(
        $(#[$m])*
        #[must_use]
        pub fn $name(self) -> Expr {
            Expr(Node::Call(Func::$f, vec![self.0]))
        }
    )*};
}

macro_rules! binary_fns {
    ($($(#[$m:meta])* $name:ident => $f:ident),* $(,)?) => {$(
        $(#[$m])*
        #[must_use]
        pub fn $name(self, other: impl Into<Expr>) -> Expr {
            Expr(Node::Call(Func::$f, vec![self.0, other.into().0]))
        }
    )*};
}

macro_rules! comparisons {
    ($($(#[$m:meta])* $name:ident => $op:ident),* $(,)?) => {$(
        $(#[$m])*
        #[must_use]
        pub fn $name(self, other: impl Into<Expr>) -> Expr {
            self.binary(BinOp::$op, other)
        }
    )*};
}

macro_rules! reductions {
    ($($(#[$m:meta])* $name:ident => $r:ident),* $(,)?) => {$(
        $(#[$m])*
        #[must_use]
        pub fn $name(self) -> Expr {
            Expr(Node::Reduce(Reduce::$r, Box::new(self.0)))
        }
    )*};
}

impl Expr {
    fn binary(self, op: BinOp, other: impl Into<Self>) -> Self {
        Self(Node::Binary(op, Box::new(self.0), Box::new(other.into().0)))
    }

    comparisons!(
        /// `self == other`.
        eq => Eq,
        /// `self != other`.
        ne => Ne,
        /// `self < other`.
        lt => Lt,
        /// `self <= other`.
        le => Le,
        /// `self > other`.
        gt => Gt,
        /// `self >= other`.
        ge => Ge,
    );

    /// `self` to the power `exponent`.
    #[must_use]
    pub fn pow(self, exponent: impl Into<Self>) -> Self {
        self.binary(BinOp::Pow, exponent)
    }

    unary_fns!(
        /// The square root.
        sqrt => Sqrt,
        /// The absolute value.
        abs => Abs,
        /// `e` to the power `self`.
        exp => Exp,
        /// The natural logarithm.
        ln => Log,
        /// The base-10 logarithm.
        log10 => Log10,
        /// The sine.
        sin => Sin,
        /// The cosine.
        cos => Cos,
        /// The tangent.
        tan => Tan,
        /// The arcsine.
        asin => Asin,
        /// The arccosine.
        acos => Acos,
        /// The arctangent.
        atan => Atan,
        /// The hyperbolic sine.
        sinh => Sinh,
        /// The hyperbolic cosine.
        cosh => Cosh,
        /// The hyperbolic tangent.
        tanh => Tanh,
        /// The largest whole number not above.
        floor => Floor,
        /// The smallest whole number not below.
        ceil => Ceil,
        /// The nearest whole number, halves away from zero.
        round => Round,
    );

    binary_fns!(
        /// `atan2(self, x)`: the angle of the point `(x, self)`.
        atan2 => Atan2,
        /// `√(self² + other²)`.
        hypot => Hypot,
        /// The smaller of the two, element by element (for the smallest object
        /// of an entry, see [`min`](Self::min)).
        min_with => Min,
        /// The larger of the two, element by element (for the largest object of
        /// an entry, see [`max`](Self::max)).
        max_with => Max,
    );

    reductions!(
        /// Whether any object of the entry is true (`false` for none).
        any => Any,
        /// Whether every object of the entry is true (`true` for none).
        all => All,
        /// How many objects of the entry are true.
        count => Count,
        /// The sum over the entry's objects (0 for none).
        sum => Sum,
        /// The largest of the entry's objects, missing for none.
        max => Max,
        /// The smallest of the entry's objects, missing for none.
        min => Min,
        /// The mean of the entry's objects, missing for none.
        mean => Mean,
        /// How many objects the entry has.
        len => Len,
    );

    /// The object at `index` of each entry (from the end for a negative
    /// one: `-1` is the last), missing where the entry has too few.
    #[must_use]
    pub fn at(self, index: i64) -> Self {
        Self(Node::At(Box::new(self.0), index))
    }

    /// The objects of each entry that `mask`, a per-object bool, keeps.
    #[must_use]
    pub fn keep(self, mask: impl Into<Self>) -> Self {
        Self(Node::Keep(Box::new(self.0), Box::new(mask.into().0)))
    }

    /// `self`, or `default` where `self` is missing.
    #[must_use]
    pub fn unwrap_or(self, default: impl Into<Self>) -> Self {
        Self(Node::UnwrapOr(Box::new(self.0), Box::new(default.into().0)))
    }

    /// The columns the expression names, once each, in order.
    #[must_use]
    pub fn columns(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.0.walk(&mut |n| {
            if let Node::Col(c) = n
                && !out.contains(&c.as_str())
            {
                out.push(c.as_str());
            }
        });
        out
    }
}

impl Node {
    pub(crate) fn children(&self) -> Vec<&Self> {
        match self {
            Self::Col(_) | Self::Lit(_) => Vec::new(),
            Self::Unary(_, a) | Self::Reduce(_, a) | Self::At(a, _) => vec![a],
            Self::Binary(_, a, b) | Self::Keep(a, b) | Self::UnwrapOr(a, b) => vec![a, b],
            Self::Call(_, args) => args.iter().collect(),
            Self::If(c, a, b) => vec![c, a, b],
        }
    }

    /// Visit this node and every node below it, parents first. The order is
    /// the one nodes are numbered in, for errors.
    pub(crate) fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Self)) {
        visit(self);
        for c in self.children() {
            c.walk(visit);
        }
    }
}

impl ops::Add for Expr {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        self.binary(BinOp::Add, rhs)
    }
}
impl ops::Sub for Expr {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self.binary(BinOp::Sub, rhs)
    }
}
impl ops::Mul for Expr {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        self.binary(BinOp::Mul, rhs)
    }
}
impl ops::Div for Expr {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        self.binary(BinOp::Div, rhs)
    }
}
impl ops::Neg for Expr {
    type Output = Self;
    fn neg(self) -> Self {
        Self(Node::Unary(UnOp::Neg, Box::new(self.0)))
    }
}
/// Logical and.
impl ops::BitAnd for Expr {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        self.binary(BinOp::And, rhs)
    }
}
/// Logical or.
impl ops::BitOr for Expr {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.binary(BinOp::Or, rhs)
    }
}
/// Logical not.
impl ops::Not for Expr {
    type Output = Self;
    fn not(self) -> Self {
        Self(Node::Unary(UnOp::Not, Box::new(self.0)))
    }
}
macro_rules! scalar_ops {
    ($($t:ty),*) => {$(
        impl ops::Add<$t> for Expr { type Output = Expr; fn add(self, rhs: $t) -> Expr { self.binary(BinOp::Add, rhs) } }
        impl ops::Sub<$t> for Expr { type Output = Expr; fn sub(self, rhs: $t) -> Expr { self.binary(BinOp::Sub, rhs) } }
        impl ops::Mul<$t> for Expr { type Output = Expr; fn mul(self, rhs: $t) -> Expr { self.binary(BinOp::Mul, rhs) } }
        impl ops::Div<$t> for Expr { type Output = Expr; fn div(self, rhs: $t) -> Expr { self.binary(BinOp::Div, rhs) } }
        impl ops::Add<Expr> for $t { type Output = Expr; fn add(self, rhs: Expr) -> Expr { Expr::from(self).binary(BinOp::Add, rhs) } }
        impl ops::Sub<Expr> for $t { type Output = Expr; fn sub(self, rhs: Expr) -> Expr { Expr::from(self).binary(BinOp::Sub, rhs) } }
        impl ops::Mul<Expr> for $t { type Output = Expr; fn mul(self, rhs: Expr) -> Expr { Expr::from(self).binary(BinOp::Mul, rhs) } }
        impl ops::Div<Expr> for $t { type Output = Expr; fn div(self, rhs: Expr) -> Expr { Expr::from(self).binary(BinOp::Div, rhs) } }
    )*};
}
scalar_ops!(f64, i64, i32);

// --- the text form -----------------------------------------------------------

impl BinOp {
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Pow => "^",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "and",
            Self::Or => "or",
        }
    }

    /// Binding strength in the text form, higher binds tighter.
    pub(crate) fn prec(self) -> u8 {
        match self {
            Self::Or => 2,
            Self::And => 3,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge => 5,
            Self::Add | Self::Sub => 7,
            Self::Mul | Self::Div => 8,
            Self::Pow => 10,
        }
    }
}

impl Func {
    pub(crate) const ALL: [(&'static str, Self, usize); 21] = [
        ("sqrt", Self::Sqrt, 1),
        ("abs", Self::Abs, 1),
        ("exp", Self::Exp, 1),
        ("log", Self::Log, 1),
        ("log10", Self::Log10, 1),
        ("sin", Self::Sin, 1),
        ("cos", Self::Cos, 1),
        ("tan", Self::Tan, 1),
        ("asin", Self::Asin, 1),
        ("acos", Self::Acos, 1),
        ("atan", Self::Atan, 1),
        ("sinh", Self::Sinh, 1),
        ("cosh", Self::Cosh, 1),
        ("tanh", Self::Tanh, 1),
        ("floor", Self::Floor, 1),
        ("ceil", Self::Ceil, 1),
        ("round", Self::Round, 1),
        ("atan2", Self::Atan2, 2),
        ("hypot", Self::Hypot, 2),
        ("min", Self::Min, 2),
        ("max", Self::Max, 2),
    ];

    pub(crate) fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, f, _)| *f == self)
            .map_or("?", |(n, _, _)| n)
    }
}

impl Reduce {
    pub(crate) const ALL: [(&'static str, Self); 8] = [
        ("any", Self::Any),
        ("all", Self::All),
        ("count", Self::Count),
        ("sum", Self::Sum),
        ("max", Self::Max),
        ("min", Self::Min),
        ("mean", Self::Mean),
        ("len", Self::Len),
    ];

    pub(crate) fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, r)| *r == self)
            .map_or("?", |(n, _)| n)
    }
}

/// Binding strength of a printed node.
fn prec(n: &Node) -> u8 {
    match n {
        Node::If(..) => 0,
        Node::Binary(op, ..) => op.prec(),
        Node::Unary(UnOp::Not, _) => 4,
        Node::UnwrapOr(..) => 6,
        Node::Unary(UnOp::Neg, _) => 9,
        Node::Lit(Lit::Int(v)) if *v < 0 => 9,
        Node::Lit(Lit::Float(v)) if v.is_sign_negative() => 9,
        _ => 11,
    }
}

/// Whether `name` can be written bare: letters, digits, `_` and `.`, not
/// starting with a digit, and not a word of the language.
fn bare(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !name.ends_with('.')
        && !crate::query::parse::KEYWORDS.contains(&name)
}

/// The text form, recording where node `target` (in [`Node::walk`] order)
/// is written, for an error to point at it.
pub(crate) struct Printer {
    pub(crate) out: String,
    counter: usize,
    target: Option<usize>,
    pub(crate) span: Option<std::ops::Range<usize>>,
}

impl Printer {
    pub(crate) fn new(target: Option<usize>) -> Self {
        Self {
            out: String::new(),
            counter: 0,
            target,
            span: None,
        }
    }

    pub(crate) fn node(&mut self, n: &Node) {
        let id = self.counter;
        self.counter += 1;
        let start = self.out.len();
        self.write(n);
        if self.target == Some(id) {
            self.span = Some(start..self.out.len());
        }
    }

    fn wrapped(&mut self, n: &Node, parens: bool) {
        if parens {
            self.out.push('(');
            self.node(n);
            self.out.push(')');
        } else {
            self.node(n);
        }
    }

    fn write(&mut self, n: &Node) {
        match n {
            Node::Col(c) => {
                if bare(c) {
                    self.out.push_str(c);
                } else {
                    let _ = write!(self.out, "`{c}`");
                }
            }
            Node::Lit(Lit::Bool(b)) => self.out.push_str(if *b { "true" } else { "false" }),
            Node::Lit(Lit::Int(v)) => {
                let _ = write!(self.out, "{v}");
            }
            Node::Lit(Lit::Float(v)) => {
                let _ = write!(self.out, "{v:?}");
            }
            Node::Unary(UnOp::Neg, a) => {
                self.out.push('-');
                let negative = matches!(**a, Node::Unary(UnOp::Neg, _))
                    || matches!(**a, Node::Lit(Lit::Int(v)) if v < 0)
                    || matches!(**a, Node::Lit(Lit::Float(v)) if v.is_sign_negative());
                self.wrapped(a, prec(a) < 9 || negative);
            }
            Node::Unary(UnOp::Not, a) => {
                self.out.push_str("not ");
                self.wrapped(a, prec(a) < 4);
            }
            Node::Binary(op, a, b) => {
                let p = op.prec();
                let comparison = p == 5;
                let (lp, rp) = if *op == BinOp::Pow {
                    (prec(a) <= p, prec(b) < p)
                } else if comparison {
                    // Comparisons do not chain: parenthesize either side.
                    (prec(a) <= p, prec(b) <= p)
                } else {
                    (prec(a) < p, prec(b) <= p)
                };
                self.wrapped(a, lp);
                let _ = write!(self.out, " {} ", op.symbol());
                self.wrapped(b, rp);
            }
            Node::Call(f, args) => {
                self.out.push_str(f.name());
                self.out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.node(a);
                }
                self.out.push(')');
            }
            Node::Reduce(r, a) => {
                self.out.push_str(r.name());
                self.out.push('(');
                self.node(a);
                self.out.push(')');
            }
            Node::At(a, i) => {
                self.wrapped(a, prec(a) < 11);
                let _ = write!(self.out, "[{i}]");
            }
            Node::Keep(a, m) => {
                self.wrapped(a, prec(a) < 11);
                self.out.push('[');
                self.node(m);
                self.out.push(']');
            }
            Node::UnwrapOr(a, d) => {
                self.wrapped(a, prec(a) <= 6);
                self.out.push_str(" ?? ");
                self.wrapped(d, prec(d) <= 6);
            }
            Node::If(c, a, b) => {
                self.out.push_str("if ");
                self.node(c);
                self.out.push_str(" then ");
                self.node(a);
                self.out.push_str(" else ");
                self.node(b);
            }
        }
    }
}

/// The text form: what [`Expr::parse`] reads back to the same expression.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer::new(None);
        p.node(&self.0);
        f.write_str(&p.out)
    }
}
