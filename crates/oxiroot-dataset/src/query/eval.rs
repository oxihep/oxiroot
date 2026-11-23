//! Evaluating a checked expression over a batch, a whole column at a time.

use std::sync::Arc;

use crate::Column;
use crate::query::expr::{BinOp, Func, Lit, Node, Reduce, UnOp};

/// Values of one kind.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    Bool(Vec<bool>),
    Int(Vec<i64>),
    Float(Vec<f64>),
}

impl Data {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Bool(v) => v.len(),
            Self::Int(v) => v.len(),
            Self::Float(v) => v.len(),
        }
    }

    pub(crate) fn floats(&self) -> Vec<f64> {
        match self {
            Self::Bool(v) => v.iter().map(|&b| f64::from(u8::from(b))).collect(),
            #[allow(clippy::cast_precision_loss, reason = "an integer used as a float")]
            Self::Int(v) => v.iter().map(|&i| i as f64).collect(),
            Self::Float(v) => v.clone(),
        }
    }

    /// `n` placeholders of this kind (zero, or false), for missing values.
    fn zeros(&self, n: usize) -> Self {
        match self {
            Self::Bool(_) => Self::Bool(vec![false; n]),
            Self::Int(_) => Self::Int(vec![0; n]),
            Self::Float(_) => Self::Float(vec![0.0; n]),
        }
    }

    /// The values at `index`, in that order.
    fn gather(&self, index: &[usize]) -> Self {
        match self {
            Self::Bool(v) => Self::Bool(index.iter().map(|&i| v[i]).collect()),
            Self::Int(v) => Self::Int(index.iter().map(|&i| v[i]).collect()),
            Self::Float(v) => Self::Float(index.iter().map(|&i| v[i]).collect()),
        }
    }
}

/// An evaluated expression: one value per entry, or (with `offsets`) one
/// per object, and which values are present.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Val {
    pub(crate) data: Data,
    /// For one value per object: entry `i` holds `data[offsets[i]..offsets[i + 1]]`.
    pub(crate) offsets: Option<Arc<Vec<u64>>>,
    /// Which values are present; `None` when all are.
    pub(crate) valid: Option<Vec<bool>>,
}

/// A problem found while evaluating node `id`.
pub(crate) struct EvalError {
    pub(crate) id: usize,
    pub(crate) message: String,
}

fn usize_of(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

impl Val {
    fn flat(data: Data) -> Self {
        Self {
            data,
            offsets: None,
            valid: None,
        }
    }

    fn present(&self, i: usize) -> bool {
        self.valid.as_ref().is_none_or(|v| v[i])
    }

    /// A literal repeated for `n` entries.
    fn literal(lit: Lit, n: usize) -> Self {
        Self::flat(match lit {
            Lit::Bool(b) => Data::Bool(vec![b; n]),
            Lit::Int(i) => Data::Int(vec![i; n]),
            Lit::Float(f) => Data::Float(vec![f; n]),
        })
    }

    /// The column's values, one per entry or one per object.
    ///
    /// # Errors
    /// A column the checker would have refused, or an unsigned 64-bit value
    /// past `i64::MAX`.
    pub(crate) fn from_column(name: &str, column: &Column) -> Result<Self, String> {
        let data = |c: &Column| -> Result<Data, String> {
            Ok(match c {
                Column::Bool(v) => Data::Bool(v.clone()),
                Column::I8(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::U8(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::I16(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::U16(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::I32(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::U32(v) => Data::Int(v.iter().map(|&x| i64::from(x)).collect()),
                Column::I64(v) => Data::Int(v.clone()),
                Column::U64(v) => Data::Int(
                    v.iter()
                        .map(|&x| i64::try_from(x))
                        .collect::<Result<_, _>>()
                        .map_err(|_| format!("column `{name}` holds a value past 2⁶³"))?,
                ),
                Column::F32(v) => Data::Float(v.iter().map(|&x| f64::from(x)).collect()),
                Column::F64(v) => Data::Float(v.clone()),
                other => return Err(format!("column `{name}` holds {}", other.type_name())),
            })
        };
        Ok(match column {
            Column::Optional { valid, values } => {
                // Spread the present values out to one slot per entry.
                let dense = data(values)?;
                let mut index = Vec::with_capacity(valid.len());
                let mut next = 0;
                for &v in valid {
                    index.push(if v { next } else { 0 });
                    next += usize::from(v);
                }
                let data = if dense.len() == 0 {
                    dense.zeros(valid.len())
                } else {
                    dense.gather(&index)
                };
                Self {
                    data,
                    offsets: None,
                    valid: Some(valid.clone()),
                }
            }
            Column::List { offsets, values } => Self {
                data: data(values)?,
                offsets: Some(Arc::new(offsets.clone())),
                valid: None,
            },
            Column::Array { len, values } => {
                let n = values.len() / (*len).max(1);
                Self {
                    data: data(values)?,
                    offsets: Some(Arc::new((0..=n).map(|i| (i * len) as u64).collect())),
                    valid: None,
                }
            }
            other => Self::flat(data(other)?),
        })
    }

    /// The member `member` of a record column, or of a list of records.
    pub(crate) fn from_member(name: &str, column: &Column, member: &str) -> Result<Self, String> {
        match column {
            Column::List { offsets, values } => {
                let m = values
                    .member(member)
                    .ok_or_else(|| format!("`{name}` has no member `{member}`"))?;
                let mut v = Self::from_column(name, m)?;
                v.offsets = Some(Arc::new(offsets.clone()));
                Ok(v)
            }
            other => {
                let m = other
                    .member(member)
                    .ok_or_else(|| format!("`{name}` has no member `{member}`"))?;
                Self::from_column(name, m)
            }
        }
    }

    /// This value as a column: `f64`, `i64` or `bool`; a list of them for one
    /// value per object; optional where the type says values can be missing
    /// (`nullable`), whether or not any is.
    pub(crate) fn into_column(self, nullable: bool) -> Column {
        let column = |d: Data| match d {
            Data::Bool(v) => Column::Bool(v),
            Data::Int(v) => Column::I64(v),
            Data::Float(v) => Column::F64(v),
        };
        let values = if nullable {
            let valid = self.valid.unwrap_or_else(|| vec![true; self.data.len()]);
            let keep: Vec<usize> = (0..valid.len()).filter(|&i| valid[i]).collect();
            Column::Optional {
                values: Box::new(column(self.data.gather(&keep))),
                valid,
            }
        } else {
            column(self.data)
        };
        match self.offsets {
            Some(o) => Column::List {
                offsets: Arc::unwrap_or_clone(o),
                values: Box::new(values),
            },
            None => values,
        }
    }

    /// The entries `mask` keeps.
    pub(crate) fn filter(&self, mask: &[bool]) -> Self {
        match &self.offsets {
            None => {
                let index: Vec<usize> = (0..mask.len()).filter(|&i| mask[i]).collect();
                Self {
                    data: self.data.gather(&index),
                    offsets: None,
                    valid: self
                        .valid
                        .as_ref()
                        .map(|v| index.iter().map(|&i| v[i]).collect()),
                }
            }
            Some(o) => {
                let mut index = Vec::new();
                let mut offsets = vec![0u64];
                for (e, &keep) in mask.iter().enumerate() {
                    if keep {
                        index.extend(usize_of(o[e])..usize_of(o[e + 1]));
                        offsets.push(index.len() as u64);
                    }
                }
                Self {
                    data: self.data.gather(&index),
                    offsets: Some(Arc::new(offsets)),
                    valid: self
                        .valid
                        .as_ref()
                        .map(|v| index.iter().map(|&i| v[i]).collect()),
                }
            }
        }
    }

    /// One value per object of `offsets`, repeating this per-entry value.
    fn broadcast(&self, offsets: &Arc<Vec<u64>>) -> Self {
        let mut index = Vec::with_capacity(usize_of(*offsets.last().unwrap_or(&0)));
        for e in 0..offsets.len() - 1 {
            for _ in offsets[e]..offsets[e + 1] {
                index.push(e);
            }
        }
        Self {
            data: self.data.gather(&index),
            offsets: Some(offsets.clone()),
            valid: self
                .valid
                .as_ref()
                .map(|v| index.iter().map(|&i| v[i]).collect()),
        }
    }
}

/// Bring values to one shape: all per entry, or all per object of the same
/// objects.
fn align(vals: &mut [Val], id: usize) -> Result<(), EvalError> {
    let Some(offsets) = vals.iter().find_map(|v| v.offsets.clone()) else {
        return Ok(());
    };
    for v in vals.iter_mut() {
        match &v.offsets {
            None => *v = v.broadcast(&offsets),
            Some(o) if Arc::ptr_eq(o, &offsets) || o == &offsets => {}
            Some(o) => {
                let entry = (0..o.len().min(offsets.len()) - 1)
                    .find(|&e| o[e + 1] - o[e] != offsets[e + 1] - offsets[e])
                    .unwrap_or(0);
                return Err(EvalError {
                    id,
                    message: format!(
                        "these have different numbers of objects: {} and {} in entry {entry} of the batch",
                        offsets[entry + 1] - offsets[entry],
                        o[entry + 1] - o[entry]
                    ),
                });
            }
        }
    }
    Ok(())
}

fn and_valid(a: &Option<Vec<bool>>, b: &Option<Vec<bool>>) -> Option<Vec<bool>> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) | (None, Some(x)) => Some(x.clone()),
        (Some(x), Some(y)) => Some(x.iter().zip(y).map(|(p, q)| *p && *q).collect()),
    }
}

/// Evaluates nodes, numbering them as the checker did.
pub(crate) struct Evaluator<'a> {
    pub(crate) entries: usize,
    pub(crate) column: &'a dyn Fn(&str) -> Result<Val, String>,
    next: usize,
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(entries: usize, column: &'a dyn Fn(&str) -> Result<Val, String>) -> Self {
        Evaluator {
            entries,
            column,
            next: 0,
        }
    }

    pub(crate) fn eval(&mut self, n: &Node) -> Result<Val, EvalError> {
        let id = self.next;
        self.next += 1;
        Ok(match n {
            Node::Col(name) => (self.column)(name).map_err(|message| EvalError { id, message })?,
            Node::Lit(lit) => Val::literal(*lit, self.entries),
            Node::Unary(op, a) => {
                let v = self.eval(a)?;
                let data = match (op, v.data) {
                    (UnOp::Neg, Data::Int(x)) => {
                        Data::Int(x.iter().map(|i| i.wrapping_neg()).collect())
                    }
                    (UnOp::Neg, Data::Float(x)) => Data::Float(x.iter().map(|f| -f).collect()),
                    (UnOp::Not, Data::Bool(x)) => Data::Bool(x.iter().map(|b| !b).collect()),
                    (_, d) => d,
                };
                Val { data, ..v }
            }
            Node::Binary(op, a, b) => {
                let mut vals = [self.eval(a)?, self.eval(b)?];
                align(&mut vals, id)?;
                let [a, b] = vals;
                binary(*op, a, b)
            }
            Node::Call(f, args) => {
                let mut vals = args
                    .iter()
                    .map(|a| self.eval(a))
                    .collect::<Result<Vec<_>, _>>()?;
                align(&mut vals, id)?;
                call(*f, vals)
            }
            Node::Reduce(r, a) => reduce(*r, self.eval(a)?),
            Node::At(a, index) => at(self.eval(a)?, *index),
            Node::Keep(a, m) => {
                let mut vals = [self.eval(a)?, self.eval(m)?];
                align(&mut vals, id)?;
                let [a, m] = vals;
                keep(a, &m)
            }
            Node::UnwrapOr(a, d) => {
                let mut vals = [self.eval(a)?, self.eval(d)?];
                align(&mut vals, id)?;
                let [a, d] = vals;
                unwrap_or(a, d)
            }
            Node::If(c, a, b) => {
                let mut vals = [self.eval(c)?, self.eval(a)?, self.eval(b)?];
                align(&mut vals, id)?;
                let [c, a, b] = vals;
                choose(&c, a, b)
            }
        })
    }
}

fn bools(v: &Val) -> &[bool] {
    match &v.data {
        Data::Bool(b) => b,
        _ => &[],
    }
}

fn binary(op: BinOp, a: Val, b: Val) -> Val {
    let offsets = a.offsets.clone();
    if matches!(op, BinOp::And | BinOp::Or) {
        // Kleene logic: `false and missing` is false, `true or missing` true.
        let (x, y) = (bools(&a), bools(&b));
        let n = x.len();
        let mut out = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for i in 0..n {
            let (pa, pb) = (a.present(i), b.present(i));
            let (va, vb) = (x[i], y[i]);
            let (value, present) = match op {
                BinOp::And => {
                    if (pa && !va) || (pb && !vb) {
                        (false, true)
                    } else {
                        (va && vb, pa && pb)
                    }
                }
                _ => {
                    if (pa && va) || (pb && vb) {
                        (true, true)
                    } else {
                        (va || vb, pa && pb)
                    }
                }
            };
            out.push(value);
            valid.push(present);
        }
        let valid = (a.valid.is_some() || b.valid.is_some()).then_some(valid);
        return Val {
            data: Data::Bool(out),
            offsets,
            valid,
        };
    }
    let valid = and_valid(&a.valid, &b.valid);
    let data = match (op, &a.data, &b.data) {
        (BinOp::Eq | BinOp::Ne, Data::Bool(x), Data::Bool(y)) => Data::Bool(
            x.iter()
                .zip(y)
                .map(|(p, q)| (p == q) == (op == BinOp::Eq))
                .collect(),
        ),
        (_, Data::Int(x), Data::Int(y)) if !matches!(op, BinOp::Div | BinOp::Pow) => {
            let int = |f: fn(i64, i64) -> i64| {
                Data::Int(x.iter().zip(y).map(|(&p, &q)| f(p, q)).collect())
            };
            let cmp = |f: fn(&i64, &i64) -> bool| {
                Data::Bool(x.iter().zip(y).map(|(p, q)| f(p, q)).collect())
            };
            match op {
                BinOp::Add => int(i64::wrapping_add),
                BinOp::Sub => int(i64::wrapping_sub),
                BinOp::Mul => int(i64::wrapping_mul),
                BinOp::Eq => cmp(i64::eq),
                BinOp::Ne => cmp(i64::ne),
                BinOp::Lt => cmp(i64::lt),
                BinOp::Le => cmp(i64::le),
                BinOp::Gt => cmp(i64::gt),
                _ => cmp(i64::ge),
            }
        }
        _ => {
            let (x, y) = (a.data.floats(), b.data.floats());
            let num = |f: fn(f64, f64) -> f64| {
                Data::Float(x.iter().zip(&y).map(|(&p, &q)| f(p, q)).collect())
            };
            let cmp = |f: fn(&f64, &f64) -> bool| {
                Data::Bool(x.iter().zip(&y).map(|(p, q)| f(p, q)).collect())
            };
            match op {
                BinOp::Add => num(|p, q| p + q),
                BinOp::Sub => num(|p, q| p - q),
                BinOp::Mul => num(|p, q| p * q),
                BinOp::Div => num(|p, q| p / q),
                BinOp::Pow => num(|p, q| {
                    if q.fract() == 0.0 && q.abs() <= 64.0 {
                        #[allow(
                            clippy::cast_possible_truncation,
                            reason = "a whole number up to 64"
                        )]
                        let n = q as i32;
                        p.powi(n)
                    } else {
                        p.powf(q)
                    }
                }),
                BinOp::Eq => cmp(f64::eq),
                BinOp::Ne => cmp(f64::ne),
                BinOp::Lt => cmp(f64::lt),
                BinOp::Le => cmp(f64::le),
                BinOp::Gt => cmp(f64::gt),
                _ => cmp(f64::ge),
            }
        }
    };
    Val {
        data,
        offsets,
        valid,
    }
}

fn call(f: Func, vals: Vec<Val>) -> Val {
    let offsets = vals[0].offsets.clone();
    let valid = vals.iter().fold(None, |acc, v| and_valid(&acc, &v.valid));
    let all_int = vals.iter().all(|v| matches!(v.data, Data::Int(_)));
    if all_int && matches!(f, Func::Abs | Func::Min | Func::Max) {
        let ints: Vec<&Vec<i64>> = vals
            .iter()
            .filter_map(|v| match &v.data {
                Data::Int(x) => Some(x),
                _ => None,
            })
            .collect();
        let data = match f {
            Func::Abs => ints[0].iter().map(|i| i.wrapping_abs()).collect(),
            Func::Min => ints[0]
                .iter()
                .zip(ints[1])
                .map(|(&p, &q)| p.min(q))
                .collect(),
            _ => ints[0]
                .iter()
                .zip(ints[1])
                .map(|(&p, &q)| p.max(q))
                .collect(),
        };
        return Val {
            data: Data::Int(data),
            offsets,
            valid,
        };
    }
    let x = vals[0].data.floats();
    let data = if vals.len() == 1 {
        let g: fn(f64) -> f64 = match f {
            Func::Sqrt => f64::sqrt,
            Func::Abs => f64::abs,
            Func::Exp => f64::exp,
            Func::Log => f64::ln,
            Func::Log10 => f64::log10,
            Func::Sin => f64::sin,
            Func::Cos => f64::cos,
            Func::Tan => f64::tan,
            Func::Asin => f64::asin,
            Func::Acos => f64::acos,
            Func::Atan => f64::atan,
            Func::Sinh => f64::sinh,
            Func::Cosh => f64::cosh,
            Func::Tanh => f64::tanh,
            Func::Floor => f64::floor,
            Func::Ceil => f64::ceil,
            _ => f64::round,
        };
        x.into_iter().map(g).collect()
    } else {
        let y = vals[1].data.floats();
        let g: fn(f64, f64) -> f64 = match f {
            Func::Atan2 => f64::atan2,
            Func::Hypot => f64::hypot,
            Func::Min => f64::min,
            _ => f64::max,
        };
        x.into_iter().zip(y).map(|(p, q)| g(p, q)).collect()
    };
    Val {
        data: Data::Float(data),
        offsets,
        valid,
    }
}

fn reduce(r: Reduce, v: Val) -> Val {
    let Some(offsets) = v.offsets.clone() else {
        return v;
    };
    let entries = offsets.len() - 1;
    let range =
        |e: usize| (usize_of(offsets[e])..usize_of(offsets[e + 1])).filter(|&i| v.present(i));
    match r {
        Reduce::Any | Reduce::All | Reduce::Count => {
            let b = bools(&v);
            match r {
                Reduce::Any => Val::flat(Data::Bool(
                    (0..entries).map(|e| range(e).any(|i| b[i])).collect(),
                )),
                Reduce::All => Val::flat(Data::Bool(
                    (0..entries).map(|e| range(e).all(|i| b[i])).collect(),
                )),
                _ => Val::flat(Data::Int(
                    (0..entries)
                        .map(|e| {
                            i64::try_from(range(e).filter(|&i| b[i]).count()).unwrap_or(i64::MAX)
                        })
                        .collect(),
                )),
            }
        }
        Reduce::Len => Val::flat(Data::Int(
            (0..entries)
                .map(|e| i64::try_from(offsets[e + 1] - offsets[e]).unwrap_or(i64::MAX))
                .collect(),
        )),
        Reduce::Sum => match &v.data {
            Data::Int(x) => Val::flat(Data::Int(
                (0..entries)
                    .map(|e| range(e).fold(0i64, |s, i| s.wrapping_add(x[i])))
                    .collect(),
            )),
            d => {
                let x = d.floats();
                Val::flat(Data::Float(
                    (0..entries).map(|e| range(e).map(|i| x[i]).sum()).collect(),
                ))
            }
        },
        Reduce::Max | Reduce::Min | Reduce::Mean => {
            let mut valid = vec![false; entries];
            let data = match (&v.data, r) {
                (Data::Int(x), Reduce::Max | Reduce::Min) => Data::Int(
                    (0..entries)
                        .map(|e| {
                            let it = range(e).map(|i| x[i]);
                            let m = if r == Reduce::Max { it.max() } else { it.min() };
                            valid[e] = m.is_some();
                            m.unwrap_or(0)
                        })
                        .collect(),
                ),
                (d, _) => {
                    let x = d.floats();
                    Data::Float(
                        (0..entries)
                            .map(|e| {
                                let vals: Vec<f64> = range(e).map(|i| x[i]).collect();
                                valid[e] = !vals.is_empty();
                                match r {
                                    Reduce::Max => {
                                        vals.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                                    }
                                    Reduce::Min => {
                                        vals.iter().copied().fold(f64::INFINITY, f64::min)
                                    }
                                    #[allow(
                                        clippy::cast_precision_loss,
                                        reason = "a count of objects"
                                    )]
                                    _ => vals.iter().sum::<f64>() / vals.len() as f64,
                                }
                            })
                            .collect(),
                    )
                }
            };
            Val {
                data,
                offsets: None,
                valid: Some(valid),
            }
        }
    }
}

fn at(v: Val, index: i64) -> Val {
    let Some(offsets) = v.offsets.clone() else {
        return v;
    };
    let entries = offsets.len() - 1;
    let mut pick = Vec::with_capacity(entries);
    let mut valid = Vec::with_capacity(entries);
    for e in 0..entries {
        let (start, len) = (offsets[e], offsets[e + 1] - offsets[e]);
        let i = if index >= 0 {
            u64::try_from(index).ok().filter(|&i| i < len)
        } else {
            len.checked_sub(index.unsigned_abs())
        };
        match i {
            Some(i) => {
                let at = usize_of(start + i);
                pick.push(at);
                valid.push(v.present(at));
            }
            None => {
                pick.push(0);
                valid.push(false);
            }
        }
    }
    let data = if v.data.len() == 0 {
        v.data.zeros(entries)
    } else {
        v.data.gather(&pick)
    };
    Val {
        data,
        offsets: None,
        valid: Some(valid),
    }
}

fn keep(v: Val, mask: &Val) -> Val {
    let Some(offsets) = v.offsets.clone() else {
        return v;
    };
    let m = bools(mask);
    let mut index = Vec::new();
    let mut out = vec![0u64];
    for e in 0..offsets.len() - 1 {
        let objects = usize_of(offsets[e])..usize_of(offsets[e + 1]);
        index.extend(objects.filter(|&i| m[i] && mask.present(i)));
        out.push(index.len() as u64);
    }
    Val {
        data: v.data.gather(&index),
        offsets: Some(Arc::new(out)),
        valid: v
            .valid
            .as_ref()
            .map(|valid| index.iter().map(|&i| valid[i]).collect()),
    }
}

fn unwrap_or(a: Val, d: Val) -> Val {
    let Some(valid) = &a.valid else {
        return a;
    };
    let pick_a: Vec<bool> = valid.clone();
    merge(&pick_a, a, d)
}

fn choose(c: &Val, a: Val, b: Val) -> Val {
    let cond = bools(c);
    let mut v = merge(cond, a, b);
    if let Some(cv) = &c.valid {
        let valid = v.valid.get_or_insert_with(|| vec![true; cv.len()]);
        for (p, &q) in valid.iter_mut().zip(cv) {
            *p = *p && q;
        }
    }
    v
}

/// `a` where `pick` is true, `b` elsewhere.
fn merge(pick: &[bool], a: Val, b: Val) -> Val {
    let offsets = a.offsets.clone().or_else(|| b.offsets.clone());
    let n = pick.len();
    let data = match (&a.data, &b.data) {
        (Data::Bool(x), Data::Bool(y)) => {
            Data::Bool((0..n).map(|i| if pick[i] { x[i] } else { y[i] }).collect())
        }
        (Data::Int(x), Data::Int(y)) => {
            Data::Int((0..n).map(|i| if pick[i] { x[i] } else { y[i] }).collect())
        }
        (x, y) => {
            let (x, y) = (x.floats(), y.floats());
            Data::Float((0..n).map(|i| if pick[i] { x[i] } else { y[i] }).collect())
        }
    };
    let valid = if a.valid.is_none() && b.valid.is_none() {
        None
    } else {
        Some(
            (0..n)
                .map(|i| if pick[i] { a.present(i) } else { b.present(i) })
                .collect(),
        )
    };
    Val {
        data,
        offsets,
        valid,
    }
}
