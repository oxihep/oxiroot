//! Types, and checking an expression against a dataset's columns before
//! anything is read.

use std::fmt;
use std::ops::Range;

use crate::DataType;
use crate::query::expr::{BinOp, Expr, Func, Lit, Node, Printer, Reduce, UnOp};

/// What a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scalar {
    Bool,
    Int,
    Float,
}

impl Scalar {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int => "integer",
            Self::Float => "float",
        }
    }
    fn numeric(self) -> bool {
        self != Self::Bool
    }
}

/// The type of an expression: its values, whether it has one value per
/// object (and of which column's objects) rather than one per entry, and
/// whether a value can be missing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ty {
    pub(crate) scalar: Scalar,
    pub(crate) objects: Option<String>,
    pub(crate) nullable: bool,
}

impl Ty {
    fn flat(scalar: Scalar) -> Self {
        Self {
            scalar,
            objects: None,
            nullable: false,
        }
    }

    fn describe(&self) -> String {
        match &self.objects {
            Some(c) => format!("one {} per object of `{c}`", self.scalar.name()),
            None => format!("one {} per entry", self.scalar.name()),
        }
    }
}

/// Where a column's values come from: a dataset column to read, maybe one
/// member of its records; or a column the query defined.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Source {
    Read {
        column: String,
        member: Option<String>,
    },
    Defined(String),
}

/// The columns an expression can name: the dataset's, with their stored
/// types, and those the query has defined so far.
pub(crate) struct Columns<'a> {
    pub(crate) dataset: &'a [(String, DataType)],
    pub(crate) defined: &'a [(String, Ty)],
}

/// What a column of `data_type` is in an expression, or why it cannot be
/// used.
pub(crate) fn column_ty(name: &str, data_type: &DataType) -> Result<Ty, String> {
    let scalar = |t: &DataType| -> Option<Scalar> {
        Some(match t {
            DataType::Bool => Scalar::Bool,
            DataType::I8
            | DataType::U8
            | DataType::I16
            | DataType::U16
            | DataType::I32
            | DataType::U32
            | DataType::I64
            | DataType::U64 => Scalar::Int,
            DataType::F32 | DataType::F64 => Scalar::Float,
            _ => return None,
        })
    };
    let unsupported = || {
        format!(
            "column `{name}` holds {data_type}: an expression takes numbers and bools, one per entry \
             or a list of them per entry"
        )
    };
    let t = data_type.storage();
    if let Some(s) = scalar(&t) {
        return Ok(Ty::flat(s));
    }
    match &t {
        DataType::Optional(inner) => scalar(inner).map(|s| Ty {
            nullable: true,
            ..Ty::flat(s)
        }),
        DataType::List(inner) | DataType::Array(_, inner) => scalar(inner).map(|s| Ty {
            scalar: s,
            objects: Some(name.to_owned()),
            nullable: false,
        }),
        _ => None,
    }
    .ok_or_else(unsupported)
}

impl Columns<'_> {
    /// Resolve `name`: a defined column, a dataset column, a member of a
    /// record column (`Muon.pt`), or the NanoAOD-style column it stands for
    /// (`Muon.pt` → `Muon_pt`).
    pub(crate) fn resolve(&self, name: &str) -> Result<(Source, Ty), (String, Option<String>)> {
        if let Some((_, ty)) = self.defined.iter().find(|(n, _)| n == name) {
            return Ok((Source::Defined(name.to_owned()), ty.clone()));
        }
        if let Some((_, t)) = self.dataset.iter().find(|(n, _)| n == name) {
            let ty = column_ty(name, t).map_err(|m| (m, None))?;
            return Ok((
                Source::Read {
                    column: name.to_owned(),
                    member: None,
                },
                ty,
            ));
        }
        if let Some((head, member)) = name.split_once('.')
            && let Some((_, t)) = self.dataset.iter().find(|(n, _)| n == head)
        {
            let (inner, listed) = match t.storage() {
                DataType::List(inner) => (*inner, true),
                other => (other, false),
            };
            if let DataType::Struct(members) = inner {
                if let Some((_, mt)) = members.iter().find(|(m, _)| m == member) {
                    let mut ty = column_ty(name, mt).map_err(|m| (m, None))?;
                    if listed {
                        if ty.objects.is_some() {
                            return Err((
                                format!(
                                    "`{name}` is a list in each object: lists of lists are not supported"
                                ),
                                None,
                            ));
                        }
                        ty.objects = Some(head.to_owned());
                    }
                    return Ok((
                        Source::Read {
                            column: head.to_owned(),
                            member: Some(member.to_owned()),
                        },
                        ty,
                    ));
                }
                let names: Vec<&str> = members.iter().map(|(m, _)| m.as_str()).collect();
                return Err((
                    format!("`{head}` has no member `{member}`"),
                    Some(format!("its members are {}", names.join(", "))),
                ));
            }
        }
        if name.contains('.') {
            let flat = name.replace('.', "_");
            if self.dataset.iter().any(|(n, _)| *n == flat) {
                return self.resolve(&flat);
            }
        }
        let hint = closest(name, self.names()).map(|c| format!("did you mean `{c}`?"));
        Err((format!("no column `{name}`"), hint))
    }

    fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.defined.iter().map(|(n, _)| n.clone()).collect();
        for (n, t) in self.dataset {
            out.push(n.clone());
            if let Some((prefix, rest)) = n.split_once('_') {
                out.push(format!("{prefix}.{rest}"));
            }
            let inner = match t.storage() {
                DataType::List(inner) => *inner,
                other => other,
            };
            if let DataType::Struct(members) = inner {
                out.extend(members.iter().map(|(m, _)| format!("{n}.{m}")));
            }
        }
        out
    }
}

/// The candidate closest to `name`, if close enough to be a typo.
pub(crate) fn closest(name: &str, candidates: Vec<String>) -> Option<String> {
    candidates
        .into_iter()
        .map(|c| {
            (
                edit_distance(&name.to_ascii_lowercase(), &c.to_ascii_lowercase()),
                c,
            )
        })
        .filter(|(d, c)| *d <= 2.max(c.len() / 4))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// A query that cannot run: what is wrong, where in which expression, and
/// how to fix it.
///
/// Its `Display` shows the expression with the problem underlined:
///
/// ```text
/// filter needs one bool per entry, but this is one bool per object of `Muon_pt`
///   nMuon >= 2 and Muon_pt > 20.0
///                  ^^^^^^^^^^^^^^
///   help: say which: any(…) or all(…)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    message: String,
    text: Option<String>,
    span: Option<Range<usize>>,
    hint: Option<String>,
}

impl QueryError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            text: None,
            span: None,
            hint: None,
        }
    }

    pub(crate) fn at(mut self, text: impl Into<String>, span: Range<usize>) -> Self {
        self.text = Some(text.into());
        self.span = Some(span);
        self
    }

    pub(crate) fn with_hint(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }

    /// What is wrong.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The expression the problem is in, as written (text) or as printed
    /// (built in Rust).
    #[must_use]
    pub fn expression(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// The byte range of [`expression`](Self::expression) the problem
    /// covers.
    #[must_use]
    pub fn span(&self) -> Option<Range<usize>> {
        self.span.clone()
    }

    /// How to fix it, where the mistake is a common one.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let (Some(text), Some(span)) = (&self.text, &self.span)
            && !text.contains('\n')
        {
            let before = text.get(..span.start).map_or(0, |s| s.chars().count());
            let width = text
                .get(span.clone())
                .map_or(1, |s| s.chars().count().max(1));
            write!(
                f,
                "\n  {text}\n  {}{}",
                " ".repeat(before),
                "^".repeat(width)
            )?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n  help: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for QueryError {}

/// A type error at node `id` (in [`Node::walk`] order).
pub(crate) struct NodeError {
    pub(crate) id: usize,
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
}

impl NodeError {
    /// As a [`QueryError`], pointing into `expr` as printed.
    pub(crate) fn into_query_error(self, expr: &Expr) -> QueryError {
        let mut p = Printer::new(Some(self.id));
        p.node(&expr.0);
        let span = p.span.unwrap_or(0..p.out.len());
        QueryError::new(self.message)
            .at(p.out, span)
            .with_hint(self.hint)
    }
}

/// What checking an expression found: its type, and the columns it reads.
pub(crate) struct Checked {
    pub(crate) ty: Ty,
    pub(crate) sources: Vec<(String, Source)>,
}

/// Check `expr` against `columns`.
pub(crate) fn check(expr: &Expr, columns: &Columns<'_>) -> Result<Checked, NodeError> {
    let mut c = Checker {
        columns,
        next: 0,
        sources: Vec::new(),
    };
    let ty = c.node(&expr.0)?;
    Ok(Checked {
        ty,
        sources: c.sources,
    })
}

struct Checker<'c, 'a> {
    columns: &'c Columns<'a>,
    next: usize,
    sources: Vec<(String, Source)>,
}

/// Combine the object-ness of two operands of an element-wise operation.
fn objects(a: &Ty, b: &Ty) -> Option<String> {
    a.objects.clone().or_else(|| b.objects.clone())
}

impl Checker<'_, '_> {
    fn node(&mut self, n: &Node) -> Result<Ty, NodeError> {
        let id = self.next;
        self.next += 1;
        let err = |message: String, hint: Option<String>| NodeError { id, message, hint };
        Ok(match n {
            Node::Col(name) => {
                let (source, ty) = self.columns.resolve(name).map_err(|(m, h)| err(m, h))?;
                if !self.sources.iter().any(|(n, _)| n == name) {
                    self.sources.push((name.clone(), source));
                }
                ty
            }
            Node::Lit(Lit::Bool(_)) => Ty::flat(Scalar::Bool),
            Node::Lit(Lit::Int(_)) => Ty::flat(Scalar::Int),
            Node::Lit(Lit::Float(_)) => Ty::flat(Scalar::Float),
            Node::Unary(op, a) => {
                let t = self.node(a)?;
                match op {
                    UnOp::Neg if !t.scalar.numeric() => {
                        return Err(err(
                            format!("`-` needs a number, not {}", t.describe()),
                            None,
                        ));
                    }
                    UnOp::Not if t.scalar != Scalar::Bool => {
                        return Err(err(
                            format!("`not` needs a bool, not {}", t.describe()),
                            Some("compare it first: `x != 0`".into()),
                        ));
                    }
                    _ => t,
                }
            }
            Node::Binary(op, a, b) => {
                let (ta, tb) = (self.node(a)?, self.node(b)?);
                let objects = objects(&ta, &tb);
                let nullable = ta.nullable || tb.nullable;
                let scalar = match op {
                    BinOp::And | BinOp::Or => {
                        for t in [&ta, &tb] {
                            if t.scalar != Scalar::Bool {
                                return Err(err(
                                    format!("`{}` needs bools, not {}", op.symbol(), t.describe()),
                                    Some("compare it first: `x > 0`".into()),
                                ));
                            }
                        }
                        Scalar::Bool
                    }
                    BinOp::Eq | BinOp::Ne => {
                        if (ta.scalar == Scalar::Bool) != (tb.scalar == Scalar::Bool) {
                            return Err(err(
                                format!(
                                    "`{}` compares a {} with a {}",
                                    op.symbol(),
                                    ta.scalar.name(),
                                    tb.scalar.name()
                                ),
                                None,
                            ));
                        }
                        Scalar::Bool
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        self.numbers(op.symbol(), &ta, &tb, id)?;
                        Scalar::Bool
                    }
                    BinOp::Div | BinOp::Pow => {
                        self.numbers(op.symbol(), &ta, &tb, id)?;
                        Scalar::Float
                    }
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        self.numbers(op.symbol(), &ta, &tb, id)?;
                        if ta.scalar == Scalar::Int && tb.scalar == Scalar::Int {
                            Scalar::Int
                        } else {
                            Scalar::Float
                        }
                    }
                };
                Ty {
                    scalar,
                    objects,
                    nullable,
                }
            }
            Node::Call(f, args) => {
                let tys = args
                    .iter()
                    .map(|a| self.node(a))
                    .collect::<Result<Vec<_>, _>>()?;
                for t in &tys {
                    if !t.scalar.numeric() {
                        return Err(err(
                            format!("`{}` needs numbers, not {}", f.name(), t.describe()),
                            None,
                        ));
                    }
                }
                let objects = tys.iter().find_map(|t| t.objects.clone());
                let nullable = tys.iter().any(|t| t.nullable);
                let all_int = tys.iter().all(|t| t.scalar == Scalar::Int);
                let scalar = match f {
                    Func::Abs | Func::Min | Func::Max if all_int => Scalar::Int,
                    _ => Scalar::Float,
                };
                Ty {
                    scalar,
                    objects,
                    nullable,
                }
            }
            Node::Reduce(r, a) => {
                let t = self.node(a)?;
                if t.objects.is_none() {
                    return Err(err(
                        format!(
                            "`{}` reduces the objects of each entry, but this is {}",
                            r.name(),
                            t.describe()
                        ),
                        None,
                    ));
                }
                let need_bool = matches!(r, Reduce::Any | Reduce::All | Reduce::Count);
                if need_bool && t.scalar != Scalar::Bool {
                    return Err(err(
                        format!(
                            "`{}` needs a bool per object, not {}",
                            r.name(),
                            t.describe()
                        ),
                        Some(format!("compare first: `{}(x > 20)`", r.name())),
                    ));
                }
                if matches!(r, Reduce::Sum | Reduce::Max | Reduce::Min | Reduce::Mean)
                    && !t.scalar.numeric()
                {
                    return Err(err(
                        format!("`{}` needs numbers, not {}", r.name(), t.describe()),
                        Some("count true objects with `count(…)`".into()),
                    ));
                }
                let (scalar, nullable) = match r {
                    Reduce::Any | Reduce::All => (Scalar::Bool, false),
                    Reduce::Count | Reduce::Len => (Scalar::Int, false),
                    Reduce::Sum => (t.scalar, false),
                    Reduce::Max | Reduce::Min => (t.scalar, true),
                    Reduce::Mean => (Scalar::Float, true),
                };
                Ty {
                    scalar,
                    objects: None,
                    nullable,
                }
            }
            Node::At(a, _) => {
                let t = self.node(a)?;
                if t.objects.is_none() {
                    return Err(err(
                        format!(
                            "`[i]` picks an object of each entry, but this is {}",
                            t.describe()
                        ),
                        None,
                    ));
                }
                Ty {
                    scalar: t.scalar,
                    objects: None,
                    nullable: true,
                }
            }
            Node::Keep(a, m) => {
                let (ta, tm) = (self.node(a)?, self.node(m)?);
                if ta.objects.is_none() || tm.objects.is_none() || tm.scalar != Scalar::Bool {
                    return Err(err(
                        format!(
                            "`x[mask]` keeps objects: `x` and `mask` need one value per object, \
                             and `mask` a bool; here they are {} and {}",
                            ta.describe(),
                            tm.describe()
                        ),
                        None,
                    ));
                }
                ta
            }
            Node::UnwrapOr(a, d) => {
                let (ta, td) = (self.node(a)?, self.node(d)?);
                let scalar = self.common(&ta, &td, "??", id)?;
                Ty {
                    scalar,
                    objects: objects(&ta, &td),
                    nullable: td.nullable,
                }
            }
            Node::If(c, a, b) => {
                let tc = self.node(c)?;
                if tc.scalar != Scalar::Bool {
                    return Err(err(
                        format!("`if` needs a bool condition, not {}", tc.describe()),
                        None,
                    ));
                }
                let (ta, tb) = (self.node(a)?, self.node(b)?);
                let scalar = self.common(&ta, &tb, "if … then … else", id)?;
                Ty {
                    scalar,
                    objects: tc.objects.clone().or_else(|| objects(&ta, &tb)),
                    nullable: tc.nullable || ta.nullable || tb.nullable,
                }
            }
        })
    }

    fn numbers(&self, op: &str, a: &Ty, b: &Ty, id: usize) -> Result<(), NodeError> {
        for t in [a, b] {
            if !t.scalar.numeric() {
                return Err(NodeError {
                    id,
                    message: format!("`{op}` needs numbers, not {}", t.describe()),
                    hint: Some("a bool is not a number: `if c then 1 else 0` makes one".into()),
                });
            }
        }
        Ok(())
    }

    /// The type two branches share (an integer and a float make a float).
    fn common(&self, a: &Ty, b: &Ty, what: &str, id: usize) -> Result<Scalar, NodeError> {
        match (a.scalar, b.scalar) {
            (x, y) if x == y => Ok(x),
            (Scalar::Int, Scalar::Float) | (Scalar::Float, Scalar::Int) => Ok(Scalar::Float),
            (x, y) => Err(NodeError {
                id,
                message: format!("`{what}` mixes a {} and a {}", x.name(), y.name()),
                hint: None,
            }),
        }
    }
}
