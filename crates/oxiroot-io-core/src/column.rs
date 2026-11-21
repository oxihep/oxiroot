//! [`Column`]: one column's values, stored flat, Arrow style, and the typed
//! views over it. It is what the tree and `RNTuple` readers return and their
//! writers take, and what a dataset's batch holds.

use crate::cast::SaturatingFrom;
use crate::{Error, Result};

/// One column's values for every entry of a batch, stored flat.
///
/// Variable-length shapes keep cumulative `offsets` (one per entry plus a
/// leading `0`) into a single child, so a column of `std::vector<float>`
/// is two buffers however many entries it has. The variants' fields are
/// public for code that builds or walks columns directly; a
/// batch checks their invariants when a column is added.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Column {
    /// `bool`.
    Bool(Vec<bool>),
    /// `i8`.
    I8(Vec<i8>),
    /// `u8`.
    U8(Vec<u8>),
    /// `i16`.
    I16(Vec<i16>),
    /// `u16`.
    U16(Vec<u16>),
    /// `i32`.
    I32(Vec<i32>),
    /// `u32`.
    U32(Vec<u32>),
    /// `i64`.
    I64(Vec<i64>),
    /// `u64`.
    U64(Vec<u64>),
    /// `f32`.
    F32(Vec<f32>),
    /// `f64`.
    F64(Vec<f64>),
    /// One string per entry: entry `i` is
    /// `text[offsets[i]..offsets[i + 1]]`.
    String {
        /// Cumulative byte boundaries, one per entry plus a leading `0`.
        /// Each falls between two characters of `text`.
        offsets: Vec<u64>,
        /// Every entry's text, back to back.
        text: String,
    },
    /// A variable number of values per entry: entry `i` is
    /// `values[offsets[i]..offsets[i + 1]]`.
    List {
        /// Cumulative boundaries into `values`, one per entry plus a leading `0`.
        offsets: Vec<u64>,
        /// Every entry's values, back to back.
        values: Box<Self>,
    },
    /// `len` values per entry: entry `i` is `values[i * len..(i + 1) * len]`.
    Array {
        /// Values per entry.
        len: usize,
        /// Every entry's values, back to back.
        values: Box<Self>,
    },
    /// An optional value per entry: `valid[i]` says whether entry `i` has one,
    /// and `values` holds only the present values, in entry order.
    Optional {
        /// Whether each entry holds a value.
        valid: Vec<bool>,
        /// The present values, densely packed.
        values: Box<Self>,
    },
    /// A record per entry: named member columns, each with one value per
    /// entry (a struct of arrays). An `RNTuple` record or user class; a
    /// `std::vector<Struct>` is a [`List`](Self::List) of these.
    Struct(Vec<(String, Self)>),
    /// One of several alternatives per entry, a `std::variant`: entry `i`
    /// holds alternative `tags[i] - 1`, at position `indices[i]` of that
    /// alternative's column, or nothing when `tags[i]` is `0`. Each
    /// alternative's column holds only the entries that chose it.
    Union {
        /// Per entry: the alternative it holds, counted from 1; `0` for none.
        tags: Vec<u32>,
        /// Per entry: where its value is in the alternative's column.
        indices: Vec<u64>,
        /// The alternatives, in order, each with the values of the entries
        /// that hold it.
        alternatives: Vec<(String, Self)>,
    },
}

/// Apply `$body` to the vector inside any scalar variant (bound as `$v`), and
/// rebuild the same variant from its result. The arms in braces handle the
/// other variants: together the match is exhaustive, so a new variant is a
/// compile error here, not a panic.
macro_rules! map_scalar {
    ($col:expr, $v:ident => $body:expr, { $($others:tt)* }) => {
        match $col {
            Column::Bool($v) => Column::Bool($body),
            Column::I8($v) => Column::I8($body),
            Column::U8($v) => Column::U8($body),
            Column::I16($v) => Column::I16($body),
            Column::U16($v) => Column::U16($body),
            Column::I32($v) => Column::I32($body),
            Column::U32($v) => Column::U32($body),
            Column::I64($v) => Column::I64($body),
            Column::U64($v) => Column::U64($body),
            Column::F32($v) => Column::F32($body),
            Column::F64($v) => Column::F64($body),
            $($others)*
        }
    };
}

fn to_usize(n: u64) -> usize {
    usize::saturating_from(n)
}

impl Column {
    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Bool(v) => v.len(),
            Self::I8(v) => v.len(),
            Self::U8(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::String { offsets, .. } | Self::List { offsets, .. } => {
                offsets.len().saturating_sub(1)
            }
            Self::Array { len, values } => values.len().checked_div(*len).unwrap_or(0),
            Self::Optional { valid, .. } => valid.len(),
            Self::Struct(members) => members.first().map_or(0, |(_, c)| c.len()),
            Self::Union { tags, .. } => tags.len(),
        }
    }

    /// The values as a slice of `T`, if this is a scalar column of that type.
    #[must_use]
    pub fn values<T: Primitive>(&self) -> Option<&[T]> {
        T::slice(self)
    }

    /// A list of structs, from one list per member: `members` are list
    /// columns with the same entries, and the result has one struct per
    /// element, its members the lists' values.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if there is no member, a member is not a list,
    /// or two members do not have the same number of values in every entry.
    pub fn list_of_structs<N: Into<String>>(
        members: impl IntoIterator<Item = (N, Self)>,
    ) -> Result<Self> {
        let mut shared: Option<Vec<u64>> = None;
        let mut fields = Vec::new();
        for (name, column) in members {
            let name = name.into();
            let Self::List { offsets, values } = column else {
                return Err(Error::InvalidInput(format!(
                    "member {name:?} is a {}, not a list",
                    column.type_name()
                )));
            };
            match &shared {
                Some(first) if *first != offsets => {
                    return Err(Error::InvalidInput(format!(
                        "member {name:?} does not have the first member's number of values \
                         in every entry"
                    )));
                }
                Some(_) => {}
                None => shared = Some(offsets),
            }
            fields.push((name, *values));
        }
        let offsets = shared
            .ok_or_else(|| Error::InvalidInput("a list of structs needs a member".into()))?;
        Ok(Self::List {
            offsets,
            values: Box::new(Self::Struct(fields)),
        })
    }

    /// The member `name` of a struct column, if this is one and it has it.
    #[must_use]
    pub fn member(&self, name: &str) -> Option<&Self> {
        match self {
            Self::Struct(members) => members.iter().find(|(n, _)| n == name).map(|(_, c)| c),
            _ => None,
        }
    }

    /// Whether the column has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The column's type as Rust spells it: `f32`, `String`, `Vec<f32>`,
    /// `[f64; 3]`, `Option<i32>`, `Vec<Vec<i64>>`.
    #[must_use]
    pub fn type_name(&self) -> String {
        match self {
            Self::Bool(_) => "bool".into(),
            Self::I8(_) => "i8".into(),
            Self::U8(_) => "u8".into(),
            Self::I16(_) => "i16".into(),
            Self::U16(_) => "u16".into(),
            Self::I32(_) => "i32".into(),
            Self::U32(_) => "u32".into(),
            Self::I64(_) => "i64".into(),
            Self::U64(_) => "u64".into(),
            Self::F32(_) => "f32".into(),
            Self::F64(_) => "f64".into(),
            Self::String { .. } => "String".into(),
            Self::List { values, .. } => format!("Vec<{}>", values.type_name()),
            Self::Array { len, values } => format!("[{}; {len}]", values.type_name()),
            Self::Optional { values, .. } => format!("Option<{}>", values.type_name()),
            Self::Struct(members) => {
                let inner: Vec<String> = members
                    .iter()
                    .map(|(n, c)| format!("{n}: {}", c.type_name()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Self::Union { alternatives, .. } => {
                let inner: Vec<String> = alternatives.iter().map(|(_, c)| c.type_name()).collect();
                format!("variant<{}>", inner.join(", "))
            }
        }
    }

    /// Roughly how many bytes the column's values take: what a writer counts
    /// to decide when a cluster is full.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Bool(v) => v.len(),
            Self::I8(v) => v.len(),
            Self::U8(v) => v.len(),
            Self::I16(v) => 2 * v.len(),
            Self::U16(v) => 2 * v.len(),
            Self::I32(v) => 4 * v.len(),
            Self::U32(v) => 4 * v.len(),
            Self::F32(v) => 4 * v.len(),
            Self::I64(v) => 8 * v.len(),
            Self::U64(v) => 8 * v.len(),
            Self::F64(v) => 8 * v.len(),
            Self::String { offsets, text } => 8 * offsets.len() + text.len(),
            Self::List { offsets, values } => 8 * offsets.len() + values.byte_size(),
            Self::Array { values, .. } => values.byte_size(),
            Self::Optional { valid, values } => valid.len() + values.byte_size(),
            Self::Struct(members) => members.iter().map(|(_, c)| c.byte_size()).sum(),
            Self::Union {
                tags, alternatives, ..
            } => {
                12 * tags.len()
                    + alternatives
                        .iter()
                        .map(|(_, c)| c.byte_size())
                        .sum::<usize>()
            }
        }
    }

    /// Check the column's invariants: offsets start at `0`, never decrease
    /// and end at the child's length; an array's child holds a whole number of
    /// entries; an optional's child holds one value per valid entry; a
    /// string's offsets fall between characters; a union's entries point inside their
    /// alternatives.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`], naming the column `name`, for the first
    /// invariant that does not hold.
    pub fn validate(&self, name: &str) -> Result<()> {
        let bad = |why: String| Err(Error::InvalidInput(format!("column {name:?}: {why}")));
        let check_offsets = |offsets: &[u64], inner: usize| -> Result<()> {
            if offsets.first() != Some(&0) {
                return bad("offsets must start with 0".into());
            }
            if offsets.windows(2).any(|w| w[0] > w[1]) {
                return bad("offsets must not decrease".into());
            }
            if offsets.last().map(|&o| to_usize(o)) != Some(inner) {
                return bad(format!("offsets must end at {inner}, the number of values"));
            }
            Ok(())
        };
        match self {
            Self::String { offsets, text } => {
                check_offsets(offsets, text.len())?;
                if offsets.iter().any(|&o| !text.is_char_boundary(to_usize(o))) {
                    return bad("an entry starts or ends inside a character".into());
                }
                Ok(())
            }
            Self::List { offsets, values } => {
                check_offsets(offsets, values.len())?;
                values.validate(name)
            }
            Self::Array { len, values } => {
                // A zero width only fits no values (an empty TTree cannot
                // record the width of a fixed array).
                if (*len == 0 && !values.is_empty()) || (*len > 0 && values.len() % len != 0) {
                    return bad(format!(
                        "{} values do not make whole arrays of {len}",
                        values.len()
                    ));
                }
                values.validate(name)
            }
            Self::Optional { valid, values } => {
                let present = valid.iter().filter(|&&v| v).count();
                if present != values.len() {
                    return bad(format!(
                        "{present} valid entries but {} values",
                        values.len()
                    ));
                }
                values.validate(name)
            }
            Self::Struct(members) => {
                let n = self.len();
                for (m, c) in members {
                    if c.len() != n {
                        return bad(format!("member {m:?} has {} entries, not {n}", c.len()));
                    }
                    c.validate(&format!("{name}.{m}"))?;
                }
                Ok(())
            }
            Self::Union {
                tags,
                indices,
                alternatives,
            } => {
                if tags.len() != indices.len() {
                    return bad(format!("{} tags but {} indices", tags.len(), indices.len()));
                }
                for (&tag, &index) in tags.iter().zip(indices) {
                    let Some(k) = (tag as usize).checked_sub(1) else {
                        continue; // holds nothing
                    };
                    match alternatives.get(k) {
                        Some((_, c)) if to_usize(index) < c.len() => {}
                        Some(_) => return bad(format!("alternative {tag} has no value {index}")),
                        None => return bad(format!("no alternative {tag}")),
                    }
                }
                for (m, c) in alternatives {
                    c.validate(&format!("{name}.{m}"))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Entries `[start, end)` as a new column (clamped to the column).
    ///
    /// Never panics, whatever the column holds: a column built by hand whose
    /// offsets do not match its values (which writing it would reject) is
    /// sliced as far as its values go.
    #[must_use]
    pub fn slice(&self, start: usize, end: usize) -> Self {
        /// The offsets of entries `start..end`, rebased to start at 0, and the
        /// range of the child they cover, clamped to the child's `len`.
        fn window(
            offsets: &[u64],
            start: usize,
            end: usize,
            len: usize,
        ) -> (Vec<u64>, usize, usize) {
            let window = offsets.get(start..=end).unwrap_or(&[]);
            let base = window.first().copied().unwrap_or(0);
            let top = window.last().copied().unwrap_or(base);
            let lo = to_usize(base).min(len);
            let hi = to_usize(top).clamp(lo, len);
            let rebased = if window.is_empty() {
                vec![0]
            } else {
                window.iter().map(|&o| o.saturating_sub(base)).collect()
            };
            (rebased, lo, hi)
        }

        let end = end.min(self.len());
        let start = start.min(end);
        map_scalar!(self, v => v[start..end].to_vec(), {
            Self::String { offsets, text } => {
                let (offsets, lo, hi) = window(offsets, start, end, text.len());
                Self::String {
                    offsets,
                    // Empty for a window that cuts a character, which only
                    // a column no batch accepts can have.
                    text: text.get(lo..hi).unwrap_or_default().to_owned(),
                }
            }
            Self::List { offsets, values } => {
                let (offsets, lo, hi) = window(offsets, start, end, values.len());
                Self::List {
                    offsets,
                    values: Box::new(values.slice(lo, hi)),
                }
            }
            Self::Array { len, values } => Self::Array {
                len: *len,
                values: Box::new(values.slice(start.saturating_mul(*len), end.saturating_mul(*len))),
            },
            Self::Optional { valid, values } => {
                let lo = valid[..start].iter().filter(|&&v| v).count();
                let hi = lo + valid[start..end].iter().filter(|&&v| v).count();
                Self::Optional {
                    valid: valid[start..end].to_vec(),
                    values: Box::new(values.slice(lo, hi)),
                }
            }
            Self::Struct(members) => Self::Struct(
                members.iter().map(|(n, c)| (n.clone(), c.slice(start, end))).collect(),
            ),
            // The alternatives stay whole: the kept entries still point into them.
            Self::Union { tags, indices, alternatives } => Self::Union {
                tags: tags.get(start..end).unwrap_or(&[]).to_vec(),
                indices: indices.get(start..end).unwrap_or(&[]).to_vec(),
                alternatives: alternatives.clone(),
            },
        })
    }

    /// The entries whose `mask` value is `true`, as a new column. `mask` must
    /// have one value per entry.
    #[must_use]
    pub fn filter(&self, mask: &[bool]) -> Self {
        fn keep<T: Copy>(v: &[T], mask: &[bool]) -> Vec<T> {
            v.iter()
                .zip(mask)
                .filter(|&(_, &m)| m)
                .map(|(&x, _)| x)
                .collect()
        }
        map_scalar!(self, v => keep(v, mask), {
            Self::String { offsets, text } => {
                let mut out_off = vec![0u64];
                let mut out = String::new();
                for (w, _) in offsets.windows(2).zip(mask).filter(|&(_, &m)| m) {
                    out.push_str(&text[to_usize(w[0])..to_usize(w[1])]);
                    out_off.push(out.len() as u64);
                }
                Self::String { offsets: out_off, text: out }
            }
            Self::List { offsets, values } => {
                let mut out_off = vec![0u64];
                let mut child = Vec::with_capacity(values.len());
                for (w, &m) in offsets.windows(2).zip(mask) {
                    let n = to_usize(w[1] - w[0]);
                    child.extend(std::iter::repeat_n(m, n));
                    if m {
                        out_off.push(out_off.last().copied().unwrap_or(0) + (w[1] - w[0]));
                    }
                }
                Self::List { offsets: out_off, values: Box::new(values.filter(&child)) }
            }
            Self::Array { len, values } => {
                let child: Vec<bool> = mask.iter().flat_map(|&m| std::iter::repeat_n(m, *len)).collect();
                Self::Array { len: *len, values: Box::new(values.filter(&child)) }
            }
            Self::Optional { valid, values } => {
                let child: Vec<bool> = valid.iter().zip(mask).filter(|&(&v, _)| v).map(|(_, &m)| m).collect();
                Self::Optional { valid: keep(valid, mask), values: Box::new(values.filter(&child)) }
            }
            Self::Struct(members) => Self::Struct(
                members.iter().map(|(n, c)| (n.clone(), c.filter(mask))).collect(),
            ),
            Self::Union { tags, indices, alternatives } => Self::Union {
                tags: keep(tags, mask),
                indices: keep(indices, mask),
                alternatives: alternatives.clone(),
            },
        })
    }

    /// Append `other`'s entries; it must be the same type.
    ///
    /// # Errors
    ///
    /// `other`, handed back, when it is another type than this column. This
    /// column may then hold part of it; the caller compares types first when
    /// that matters.
    pub fn append(&mut self, other: Self) -> std::result::Result<(), Self> {
        fn rebase(offsets: &mut Vec<u64>, more: &[u64]) {
            let base = offsets.last().copied().unwrap_or(0);
            offsets.extend(more.iter().skip(1).map(|&o| o + base));
        }
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a.extend(b),
            (Self::I8(a), Self::I8(b)) => a.extend(b),
            (Self::U8(a), Self::U8(b)) => a.extend(b),
            (Self::I16(a), Self::I16(b)) => a.extend(b),
            (Self::U16(a), Self::U16(b)) => a.extend(b),
            (Self::I32(a), Self::I32(b)) => a.extend(b),
            (Self::U32(a), Self::U32(b)) => a.extend(b),
            (Self::I64(a), Self::I64(b)) => a.extend(b),
            (Self::U64(a), Self::U64(b)) => a.extend(b),
            (Self::F32(a), Self::F32(b)) => a.extend(b),
            (Self::F64(a), Self::F64(b)) => a.extend(b),
            (
                Self::String { offsets, text },
                Self::String {
                    offsets: o2,
                    text: t2,
                },
            ) => {
                rebase(offsets, &o2);
                text.push_str(&t2);
            }
            (
                Self::List { offsets, values },
                Self::List {
                    offsets: o2,
                    values: v2,
                },
            ) => {
                values.append(*v2)?;
                rebase(offsets, &o2);
            }
            (
                Self::Array { len, values },
                Self::Array {
                    len: l2,
                    values: v2,
                },
            ) if *len == l2 => {
                values.append(*v2)?;
            }
            (
                Self::Optional { valid, values },
                Self::Optional {
                    valid: m2,
                    values: v2,
                },
            ) => {
                values.append(*v2)?;
                valid.extend(m2);
            }
            (Self::Struct(members), Self::Struct(more))
                if members.len() == more.len()
                    && members.iter().zip(&more).all(|((a, _), (b, _))| a == b) =>
            {
                for ((_, c), (_, m)) in members.iter_mut().zip(more) {
                    c.append(m)?;
                }
            }
            (
                Self::Union {
                    tags,
                    indices,
                    alternatives,
                },
                Self::Union {
                    tags: t2,
                    indices: i2,
                    alternatives: more,
                },
            ) if alternatives.len() == more.len()
                && alternatives
                    .iter()
                    .zip(&more)
                    .all(|((a, _), (b, _))| a == b) =>
            {
                // The appended entries point past what each alternative held.
                let held: Vec<u64> = alternatives.iter().map(|(_, c)| c.len() as u64).collect();
                for (&tag, index) in t2.iter().zip(i2) {
                    let base = (tag as usize).checked_sub(1).and_then(|k| held.get(k));
                    indices.push(index + base.copied().unwrap_or(0));
                }
                tags.extend(t2);
                for ((_, c), (_, m)) in alternatives.iter_mut().zip(more) {
                    c.append(m)?;
                }
            }
            (_, other) => return Err(other),
        }
        Ok(())
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A scalar type a column can hold: `bool`, `i8` … `u64`, `f32`, `f64`.
pub trait Primitive: Copy + sealed::Sealed + 'static {
    /// The type's name, as in [`Column::type_name`].
    const NAME: &'static str;

    /// The values, if `column` holds this type.
    fn slice(column: &Column) -> Option<&[Self]>;

    /// A column of these values.
    fn into_column(values: Vec<Self>) -> Column;
}

macro_rules! primitive {
    ($($t:ty => $variant:ident),* $(,)?) => {$(
        impl sealed::Sealed for $t {}
        impl Primitive for $t {
            const NAME: &'static str = stringify!($t);
            fn slice(column: &Column) -> Option<&[Self]> {
                match column {
                    Column::$variant(v) => Some(v),
                    _ => None,
                }
            }
            fn into_column(values: Vec<Self>) -> Column {
                Column::$variant(values)
            }
        }
    )*};
}

primitive! {
    bool => Bool, i8 => I8, u8 => U8, i16 => I16, u16 => U16, i32 => I32,
    u32 => U32, i64 => I64, u64 => U64, f32 => F32, f64 => F64,
}

/// Cumulative offsets (with a leading `0`) for rows of these lengths.
fn offsets_of(lengths: impl Iterator<Item = usize>) -> Vec<u64> {
    let mut offsets = vec![0u64];
    let mut acc = 0u64;
    for n in lengths {
        acc += n as u64;
        offsets.push(acc);
    }
    offsets
}

fn strings_column<S: AsRef<str>>(values: &[S]) -> Column {
    let offsets = offsets_of(values.iter().map(|s| s.as_ref().len()));
    let mut text = String::with_capacity(offsets.last().map_or(0, |&o| to_usize(o)));
    for s in values {
        text.push_str(s.as_ref());
    }
    Column::String { offsets, text }
}

// What becomes a [`Column`] with `Column::from` or `.into()`: `Vec<T>`,
// `Vec<Vec<T>>`, `Vec<Vec<Vec<T>>>`, `Vec<[T; N]>` and `Vec<Option<T>>` of a
// [`Primitive`] `T`; `Vec<String>`, `Vec<&str>` and `Vec<Vec<String>>`.

impl<T: Primitive> From<Vec<T>> for Column {
    fn from(data: Vec<T>) -> Self {
        T::into_column(data)
    }
}

impl<T: Primitive> From<Vec<Vec<T>>> for Column {
    fn from(data: Vec<Vec<T>>) -> Self {
        let offsets = offsets_of(data.iter().map(Vec::len));
        Self::List {
            offsets,
            values: Box::new(T::into_column(data.concat())),
        }
    }
}

impl<T: Primitive> From<Vec<Vec<Vec<T>>>> for Column {
    fn from(data: Vec<Vec<Vec<T>>>) -> Self {
        let offsets = offsets_of(data.iter().map(Vec::len));
        Self::List {
            offsets,
            values: Box::new(Self::from(data.into_iter().flatten().collect::<Vec<_>>())),
        }
    }
}

impl<T: Primitive, const N: usize> From<Vec<[T; N]>> for Column {
    fn from(data: Vec<[T; N]>) -> Self {
        Self::Array {
            len: N,
            values: Box::new(T::into_column(data.concat())),
        }
    }
}

impl<T: Primitive> From<Vec<Option<T>>> for Column {
    fn from(data: Vec<Option<T>>) -> Self {
        Self::Optional {
            valid: data.iter().map(Option::is_some).collect(),
            values: Box::new(T::into_column(data.into_iter().flatten().collect())),
        }
    }
}

impl From<Vec<String>> for Column {
    fn from(data: Vec<String>) -> Self {
        strings_column(&data)
    }
}

impl From<Vec<&str>> for Column {
    fn from(data: Vec<&str>) -> Self {
        strings_column(&data)
    }
}

impl From<Vec<Vec<String>>> for Column {
    fn from(data: Vec<Vec<String>>) -> Self {
        let offsets = offsets_of(data.iter().map(Vec::len));
        let flat: Vec<String> = data.into_iter().flatten().collect();
        Self::List {
            offsets,
            values: Box::new(strings_column(&flat)),
        }
    }
}

/// A list column viewed in place: per-entry offsets over one slice of values.
/// From a batch's `jagged`.
#[derive(Debug, Clone, Copy)]
pub struct Jagged<'a, T> {
    offsets: &'a [u64],
    values: &'a [T],
}

impl<'a, T> Jagged<'a, T> {
    /// A view of `values` cut into entries at `offsets`.
    #[must_use]
    pub fn new(offsets: &'a [u64], values: &'a [T]) -> Self {
        Jagged { offsets, values }
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Entry `i`'s values. Panics if `i` is out of range, like slice indexing.
    #[must_use]
    pub fn row(&self, i: usize) -> &'a [T] {
        &self.values[to_usize(self.offsets[i])..to_usize(self.offsets[i + 1])]
    }

    /// Every entry's values, in order.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &'a [T]> + 'a {
        let values = self.values;
        self.offsets
            .windows(2)
            .map(move |w| &values[to_usize(w[0])..to_usize(w[1])])
    }

    /// All values, back to back.
    #[must_use]
    pub fn values(&self) -> &'a [T] {
        self.values
    }

    /// The cumulative offsets: one per entry plus a leading `0`.
    #[must_use]
    pub fn offsets(&self) -> &'a [u64] {
        self.offsets
    }
}

/// A string column viewed in place. From
/// a batch's `strings`.
#[derive(Debug, Clone, Copy)]
pub struct Strings<'a> {
    offsets: &'a [u64],
    text: &'a str,
}

impl<'a> Strings<'a> {
    /// A view of `text` cut into entries at `offsets`.
    #[must_use]
    pub fn new(offsets: &'a [u64], text: &'a str) -> Self {
        Strings { offsets, text }
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Entry `i`'s string. Panics if `i` is out of range, like slice indexing.
    #[must_use]
    pub fn get(&self, i: usize) -> &'a str {
        &self.text[to_usize(self.offsets[i])..to_usize(self.offsets[i + 1])]
    }

    /// Every entry's string, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a str> + 'a {
        let s = *self;
        (0..s.len()).map(move |i| s.get(i))
    }
}

#[cfg(test)]
mod slice_tests {
    use super::Column;

    #[test]
    fn slicing_a_list_rebases_its_offsets() {
        let list = Column::List {
            offsets: vec![0, 2, 2, 5],
            values: Box::new(Column::I32(vec![1, 2, 3, 4, 5])),
        };
        assert_eq!(
            list.slice(1, 3),
            Column::List {
                offsets: vec![0, 0, 3],
                values: Box::new(Column::I32(vec![3, 4, 5])),
            }
        );
    }

    /// Columns no writer would accept: slicing them must still not panic.
    #[test]
    fn slicing_an_inconsistent_column_does_not_panic() {
        let no_offsets = Column::String {
            offsets: vec![],
            text: "abc".into(),
        };
        assert_eq!(no_offsets.slice(0, 4).len(), 0);

        let past_the_values = Column::List {
            offsets: vec![0, 2, 9],
            values: Box::new(Column::I32(vec![1, 2, 3])),
        };
        assert_eq!(past_the_values.slice(0, 2).len(), 2);

        let decreasing = Column::String {
            offsets: vec![3, 1, 2],
            text: "abc".into(),
        };
        assert_eq!(decreasing.slice(0, 2).len(), 2);

        let short_child = Column::Optional {
            valid: vec![true, true, true],
            values: Box::new(Column::F64(vec![1.0])),
        };
        assert_eq!(short_child.slice(1, 3).len(), 2);

        let huge_array = Column::Array {
            len: usize::MAX,
            values: Box::new(Column::U8(vec![1, 2])),
        };
        assert_eq!(huge_array.slice(0, 1).len(), 0);
    }
}
