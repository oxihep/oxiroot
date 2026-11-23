//! Typed Rust closures as query steps ([`Query::define_with`] and
//! [`Query::filter_with`]), for what an expression cannot say.
//!
//! [`Query::define_with`]: crate::Query::define_with
//! [`Query::filter_with`]: crate::Query::filter_with

use std::marker::PhantomData;
use std::sync::Arc;

use crate::query::check::{Scalar, Ty};
use crate::query::eval::{Data, Val};

mod private {
    pub trait Sealed {}
}

/// The type of one column, as a closure argument or result sees it. Opaque.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ArgType(pub(crate) Ty);

/// A closure's results over a batch. Opaque.
#[doc(hidden)]
#[derive(Debug)]
pub struct ArgValue(pub(crate) Val);

/// One column of a batch, as a closure argument reads it. Opaque.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct ArgColumn<'a>(pub(crate) &'a Val);

/// A type a closure can take as an argument: a number or `bool` (one per
/// entry), an `Option` of one (a column whose values can be missing), or a
/// `Vec` of one (one value per object of the entry).
pub trait FromValue: private::Sealed + Sized {
    #[doc(hidden)]
    fn __accepts(ty: &ArgType) -> bool;
    #[doc(hidden)]
    fn __get(column: ArgColumn<'_>, entry: usize) -> Result<Self, String>;
}

/// A type a closure can return: a number or `bool`, an `Option` of one (a
/// missing value), or a `Vec` of one (one value per object).
pub trait IntoValue: private::Sealed + Sized + 'static {
    #[doc(hidden)]
    fn __type() -> ArgType;
    #[doc(hidden)]
    fn __collect(values: Vec<Option<Self>>) -> ArgValue;
}

/// A scalar a closure takes or returns.
trait ScalarValue: Copy + 'static {
    const SCALAR: Scalar;
    /// From the value at `i`, if it fits this type.
    fn from_data(data: &Data, i: usize) -> Option<Self>;
    fn into_data(values: Vec<Self>) -> Data;
}

macro_rules! int_scalar {
    ($($t:ty),*) => {$(
        impl ScalarValue for $t {
            const SCALAR: Scalar = Scalar::Int;
            fn from_data(data: &Data, i: usize) -> Option<Self> {
                match data {
                    Data::Int(v) => <$t>::try_from(v[i]).ok(),
                    _ => None,
                }
            }
            fn into_data(values: Vec<Self>) -> Data {
                Data::Int(values.into_iter().map(|v| i64::try_from(v).unwrap_or(i64::MAX)).collect())
            }
        }
    )*};
}
int_scalar!(i8, i16, i32, i64, u8, u16, u32, u64);

impl ScalarValue for f64 {
    const SCALAR: Scalar = Scalar::Float;
    fn from_data(data: &Data, i: usize) -> Option<Self> {
        match data {
            Data::Float(v) => Some(v[i]),
            #[allow(clippy::cast_precision_loss, reason = "an integer taken as a float")]
            Data::Int(v) => Some(v[i] as Self),
            Data::Bool(_) => None,
        }
    }
    fn into_data(values: Vec<Self>) -> Data {
        Data::Float(values)
    }
}

impl ScalarValue for f32 {
    const SCALAR: Scalar = Scalar::Float;
    fn from_data(data: &Data, i: usize) -> Option<Self> {
        #[allow(clippy::cast_possible_truncation, reason = "taken as f32, as asked")]
        f64::from_data(data, i).map(|v| v as Self)
    }
    fn into_data(values: Vec<Self>) -> Data {
        Data::Float(values.into_iter().map(f64::from).collect())
    }
}

impl ScalarValue for bool {
    const SCALAR: Scalar = Scalar::Bool;
    fn from_data(data: &Data, i: usize) -> Option<Self> {
        match data {
            Data::Bool(v) => Some(v[i]),
            _ => None,
        }
    }
    fn into_data(values: Vec<Self>) -> Data {
        Data::Bool(values)
    }
}

/// Whether a scalar of `want` can be read from a column of `have`: an
/// integer type from integers, a float from floats or integers, a `bool`
/// from bools.
fn fits(want: Scalar, have: Scalar) -> bool {
    want == have || (want == Scalar::Float && have == Scalar::Int)
}

macro_rules! scalar_values {
    ($($t:ty),*) => {$(
        impl private::Sealed for $t {}
        impl FromValue for $t {
            fn __accepts(ty: &ArgType) -> bool {
                fits(<$t>::SCALAR, ty.0.scalar) && ty.0.objects.is_none() && !ty.0.nullable
            }
            fn __get(column: ArgColumn<'_>, entry: usize) -> Result<Self, String> {
                fetch::<$t>(column.0, entry)
            }
        }
        impl IntoValue for $t {
            fn __type() -> ArgType {
                ArgType(Ty { scalar: <$t>::SCALAR, objects: None, nullable: false })
            }
            fn __collect(values: Vec<Option<Self>>) -> ArgValue {
                let valid: Vec<bool> = values.iter().map(Option::is_some).collect();
                let all = valid.iter().all(|&v| v);
                let data = values.into_iter().map(|v| v.unwrap_or_else(default::<$t>)).collect();
                ArgValue(Val { data: <$t>::into_data(data), offsets: None, valid: (!all).then_some(valid) })
            }
        }
        impl private::Sealed for Option<$t> {}
        impl FromValue for Option<$t> {
            fn __accepts(ty: &ArgType) -> bool {
                fits(<$t>::SCALAR, ty.0.scalar) && ty.0.objects.is_none()
            }
            fn __get(column: ArgColumn<'_>, entry: usize) -> Result<Self, String> {
                let v = column.0;
                if v.valid.as_ref().is_some_and(|valid| !valid[entry]) {
                    return Ok(None);
                }
                fetch::<$t>(v, entry).map(Some)
            }
        }
        impl IntoValue for Option<$t> {
            fn __type() -> ArgType {
                ArgType(Ty { scalar: <$t>::SCALAR, objects: None, nullable: true })
            }
            fn __collect(values: Vec<Option<Self>>) -> ArgValue {
                <$t as IntoValue>::__collect(values.into_iter().map(Option::flatten).collect())
            }
        }
        impl private::Sealed for Vec<$t> {}
        impl FromValue for Vec<$t> {
            fn __accepts(ty: &ArgType) -> bool {
                fits(<$t>::SCALAR, ty.0.scalar) && ty.0.objects.is_some() && !ty.0.nullable
            }
            fn __get(column: ArgColumn<'_>, entry: usize) -> Result<Self, String> {
                let v = column.0;
                let Some(offsets) = v.offsets.as_ref() else {
                    return Err("not one value per object".into());
                };
                let start = usize::try_from(offsets[entry]).unwrap_or(usize::MAX);
                let end = usize::try_from(offsets[entry + 1]).unwrap_or(usize::MAX);
                (start..end).map(|i| fetch::<$t>(v, i)).collect()
            }
        }
        impl IntoValue for Vec<$t> {
            fn __type() -> ArgType {
                ArgType(Ty { scalar: <$t>::SCALAR, objects: Some("a closure's result".into()), nullable: false })
            }
            fn __collect(values: Vec<Option<Self>>) -> ArgValue {
                let mut offsets = vec![0u64];
                let mut flat = Vec::new();
                for v in values {
                    flat.extend(v.unwrap_or_default());
                    offsets.push(flat.len() as u64);
                }
                ArgValue(Val { data: <$t>::into_data(flat), offsets: Some(Arc::new(offsets)), valid: None })
            }
        }
    )*};
}

/// The value at `i` as a `T`, or why it does not fit.
fn fetch<T: ScalarValue>(v: &Val, i: usize) -> Result<T, String> {
    T::from_data(&v.data, i).ok_or_else(|| {
        let shown = match &v.data {
            Data::Int(x) => x[i].to_string(),
            Data::Float(x) => x[i].to_string(),
            Data::Bool(x) => x[i].to_string(),
        };
        format!("{shown} does not fit {}", std::any::type_name::<T>())
    })
}

/// A value for a missing slot, never read as one: zero, or false.
fn default<T: ScalarValue>() -> T {
    T::from_data(&zero_data(T::SCALAR), 0).unwrap_or_else(|| unreachable!("zero fits its own type"))
}

fn zero_data(s: Scalar) -> Data {
    match s {
        Scalar::Bool => Data::Bool(vec![false]),
        Scalar::Int => Data::Int(vec![0]),
        Scalar::Float => Data::Float(vec![0.0]),
    }
}

scalar_values!(f64, f32, i8, i16, i32, i64, u8, u16, u32, u64, bool);

/// A closure of one to six arguments, each a [`FromValue`], returning `R`.
/// Implemented for every such `Fn`; name the argument types in the
/// closure (`|pt: f32, eta: f32| …`).
pub trait Callable<Args, R>: Send + Sync + 'static {
    #[doc(hidden)]
    fn __arity() -> usize;
    #[doc(hidden)]
    fn __accepts(types: &[ArgType]) -> Option<usize>;
    #[doc(hidden)]
    fn __call(&self, columns: &[ArgColumn<'_>], entry: usize) -> Result<R, (usize, String)>;
}

macro_rules! callable {
    ($n:literal; $($t:ident $i:tt),*) => {
        impl<F, R, $($t: FromValue),*> Callable<($($t,)*), R> for F
        where
            F: Fn($($t),*) -> R + Send + Sync + 'static,
        {
            fn __arity() -> usize {
                $n
            }
            fn __accepts(types: &[ArgType]) -> Option<usize> {
                [$($t::__accepts(&types[$i])),*].iter().position(|ok| !ok)
            }
            fn __call(&self, columns: &[ArgColumn<'_>], entry: usize) -> Result<R, (usize, String)> {
                Ok(self($($t::__get(columns[$i], entry).map_err(|m| ($i, m))?),*))
            }
        }
    };
}
callable!(1; A 0);
callable!(2; A 0, B 1);
callable!(3; A 0, B 1, C 2);
callable!(4; A 0, B 1, C 2, D 3);
callable!(5; A 0, B 1, C 2, D 3, E 4);
callable!(6; A 0, B 1, C 2, D 3, E 4, G 5);

/// A closure with its types erased, as a query step holds it.
pub(crate) trait Erased: Send + Sync {
    fn arity(&self) -> usize;
    /// The first argument the column types do not fit, if one.
    fn rejects(&self, types: &[Ty]) -> Option<usize>;
    fn output(&self) -> Ty;
    /// The closure over every entry, or the first argument that does not
    /// fit, with why.
    fn run(&self, columns: &[Val], entries: usize) -> Result<Val, (usize, String)>;
    fn signature(&self) -> &'static str;
}

pub(crate) struct Closure<F, Args, R> {
    f: F,
    _types: PhantomData<fn(Args) -> R>,
}

impl<F, Args, R> Closure<F, Args, R> {
    pub(crate) fn new(f: F) -> Self {
        Self {
            f,
            _types: PhantomData,
        }
    }
}

impl<F, Args, R> Erased for Closure<F, Args, R>
where
    F: Callable<Args, R>,
    R: IntoValue,
    Args: 'static,
{
    fn arity(&self) -> usize {
        F::__arity()
    }
    fn rejects(&self, types: &[Ty]) -> Option<usize> {
        let types: Vec<ArgType> = types.iter().cloned().map(ArgType).collect();
        F::__accepts(&types)
    }
    fn output(&self) -> Ty {
        R::__type().0
    }
    fn run(&self, columns: &[Val], entries: usize) -> Result<Val, (usize, String)> {
        let args: Vec<ArgColumn<'_>> = columns.iter().map(ArgColumn).collect();
        let values = (0..entries)
            .map(|e| self.f.__call(&args, e).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(R::__collect(values).0)
    }
    fn signature(&self) -> &'static str {
        std::any::type_name::<Args>()
    }
}
