//! Lowering fields to the on-disk model: one field record per (sub)field and
//! one column with its encoded page per column.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Error, Result};

use oxiroot_io_core::{Column, DataType};

use super::check::{check_fields, entry_count};
use super::classes::class_checksum;
use super::fields::{Field, alternative_types, item_type, member_class, member_types, positional};
use super::{
    FIELD_FLAG_ARRAY, FIELD_FLAG_CHECKSUM, ROLE_COLLECTION, ROLE_LEAF, ROLE_RECORD, ROLE_VARIANT,
};
use crate::column::ColumnType;

pub(super) struct FieldPlan {
    pub(super) name: String,
    pub(super) type_name: String,
    pub(super) parent_id: u32,
    pub(super) role: u16,
    /// Field flag bits (`FIELD_FLAG_*`): array/checksum.
    pub(super) flags: u16,
    /// `std::array`/`std::bitset` element count, when [`FIELD_FLAG_ARRAY`] is set.
    pub(super) array_size: Option<u64>,
    /// ROOT class checksum, when [`FIELD_FLAG_CHECKSUM`] is set (user classes).
    pub(super) type_checksum: Option<u32>,
}

pub(super) struct ColumnPlan {
    pub(super) column_type: ColumnType,
    pub(super) bits: u16,
    pub(super) field_id: u32,
    pub(super) page: Vec<u8>,
    pub(super) n: u32,
    pub(super) value_range: Option<(f64, f64)>,
}

fn le_bytes<T, const N: usize>(values: &[T], to: impl Fn(&T) -> [u8; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * N);
    for v in values {
        out.extend_from_slice(&to(v));
    }
    out
}

fn pack_bits(v: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; v.len().div_ceil(8)];
    for (i, &b) in v.iter().enumerate() {
        if b {
            out[i >> 3] |= 1 << (i & 7);
        }
    }
    out
}

/// An Index column holds one cumulative end per entry, where a collection
/// column's public `offsets` lead with a `0`; drop it.
fn on_disk_offsets(offsets: &[u64]) -> &[u64] {
    offsets.get(1..).unwrap_or(&[])
}

/// Bit-pack `bits`-wide unsigned values LSB-first into little-endian bytes (the
/// inverse of the reader's unpacking; used by the truncated/quantized reals).
fn pack_uints(values: &[u64], bits: u16) -> Vec<u8> {
    let nbits = bits as usize;
    let mut out = vec![0u8; (values.len() * nbits).div_ceil(8)];
    let mut pos = 0usize;
    for &v in values {
        for b in 0..nbits {
            if (v >> b) & 1 != 0 {
                let g = pos + b;
                out[g >> 3] |= 1 << (g & 7);
            }
        }
        pos += nbits;
    }
    out
}

/// Encode an IEEE-754 single into a half (binary16), round-to-nearest-even.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "IEEE half-float encoding: masked bit fields"
)]
fn f32_to_half(value: f32) -> u16 {
    let x = value.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let mut mant = (x & 0x007f_ffff) as i32;
    let exp = ((x >> 23) & 0xff) as i32;

    if exp == 0xff {
        // Inf, or NaN (keep it a non-signalling NaN).
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    let he = exp - 127 + 15; // rebias to the half's exponent
    if he >= 0x1f {
        return sign | 0x7c00; // overflow -> Inf
    }
    if he <= 0 {
        if he < -10 {
            return sign; // underflow -> +/-0
        }
        // Subnormal half: shift the (restored) mantissa down, rounding to even.
        mant |= 0x0080_0000;
        let shift = 14 - he;
        let mut h = (mant >> shift) as u16;
        let rem = mant & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        if rem > halfway || (rem == halfway && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    // Normal half; rounding may carry into the exponent, which is correct.
    let mut h = (u16::saturating_from(he) << 10) | (mant >> 13) as u16;
    let rem = mant & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h
}

/// One column's lowered bytes, before its field id is known.
struct RawCol {
    column_type: ColumnType,
    bits: u16,
    page: Vec<u8>,
    n: u32,
    value_range: Option<(f64, f64)>,
}

/// A lowered field subtree — a field, its own columns, and its children —
/// before field ids are assigned by the depth-first walk in [`flatten_tree`].
struct Node {
    name: String,
    type_name: String,
    role: u16,
    cols: Vec<RawCol>,
    children: Vec<Self>,
    /// Field flag bits (`FIELD_FLAG_*`).
    flags: u16,
    /// Fixed array element count (`std::array`/`std::bitset`).
    array_size: Option<u64>,
    /// ROOT class checksum (user-class records).
    type_checksum: Option<u32>,
}

fn raw(column_type: ColumnType, bits: u16, page: Vec<u8>, n: usize) -> RawCol {
    RawCol {
        column_type,
        bits,
        page,
        n: u32::saturating_from(n),
        value_range: None,
    }
}

/// A scalar leaf: one field, one column, no children.
fn leaf_node(name: &str, type_name: &str, col: RawCol) -> Node {
    Node {
        name: name.to_string(),
        type_name: type_name.to_string(),
        role: ROLE_LEAF,
        cols: vec![col],
        children: vec![],
        flags: 0,
        array_size: None,
        type_checksum: None,
    }
}

/// A `std::string` leaf: an Index64 offset column plus a Char column.
fn string_node(name: &str, offsets: &[u64], text: &str) -> Node {
    let ends = on_disk_offsets(offsets);
    Node {
        name: name.to_string(),
        type_name: "std::string".to_string(),
        role: ROLE_LEAF,
        cols: vec![
            raw(
                ColumnType::Index64,
                64,
                le_bytes(ends, |x| x.to_le_bytes()),
                ends.len(),
            ),
            raw(ColumnType::Char, 8, text.as_bytes().to_vec(), text.len()),
        ],
        children: vec![],
        flags: 0,
        array_size: None,
        type_checksum: None,
    }
}

/// A collection field: an Index64 offset column over `offsets` plus the single
/// element `child`. Its type name is `std::vector<child>`.
fn collection_node(name: &str, offsets: &[u64], n_outer: usize, child: Node) -> Node {
    collection_node_wrapped(name, "std::vector", offsets, n_outer, child)
}

/// Like [`collection_node`] but with a caller-chosen container `wrapper`
/// (`"std::vector"`, `"std::optional"`, `"std::unique_ptr"`, …). All share the
/// same on-disk shape — an `Index64` offset column over a single child — so they
/// differ only in the field's `type_name`.
fn collection_node_wrapped(
    name: &str,
    wrapper: &str,
    offsets: &[u64],
    n_outer: usize,
    child: Node,
) -> Node {
    Node {
        name: name.to_string(),
        type_name: format!("{wrapper}<{}>", child.type_name),
        role: ROLE_COLLECTION,
        cols: vec![raw(
            ColumnType::Index64,
            64,
            le_bytes(offsets, |x| x.to_le_bytes()),
            n_outer,
        )],
        children: vec![child],
        flags: 0,
        array_size: None,
        type_checksum: None,
    }
}

/// Cumulative present-count offsets for an `optional/unique_ptr` presence mask:
/// `offsets[i]` is the number of present entries in `0..=i` (the Index64 column).
fn present_offsets(present: &[bool]) -> Vec<u64> {
    let mut acc = 0u64;
    present
        .iter()
        .map(|&p| {
            if p {
                acc += 1;
            }
            acc
        })
        .collect()
}

/// ROOT's anonymous-record type names: a 2-field record serializes as a
/// `std::pair`, more as a `std::tuple`.
fn record_type_name(children: &[Node]) -> String {
    let inner: Vec<&str> = children.iter().map(|c| c.type_name.as_str()).collect();
    match inner.as_slice() {
        [a, b] => format!("std::pair<{a},{b}>"),
        _ => format!("std::tuple<{}>", inner.join(",")),
    }
}

/// A fixed-width value that has a column type of its own.
trait Scalar: Copy {
    /// The field's type name, as ROOT spells it.
    const TYPE_NAME: &'static str;
    const COLUMN: ColumnType;
    const BITS: u16;
    /// The values as a page: little-endian, or bit-packed for `bool`.
    fn page(values: &[Self]) -> Vec<u8>;
}

macro_rules! scalar {
    ($($t:ty => $name:literal, $column:ident, $bits:literal;)*) => {$(
        impl Scalar for $t {
            const TYPE_NAME: &'static str = $name;
            const COLUMN: ColumnType = ColumnType::$column;
            const BITS: u16 = $bits;
            fn page(values: &[Self]) -> Vec<u8> {
                le_bytes(values, |x| x.to_le_bytes())
            }
        }
    )*};
}

scalar! {
    i8 => "std::int8_t", Int8, 8;
    u8 => "std::uint8_t", UInt8, 8;
    i16 => "std::int16_t", Int16, 16;
    u16 => "std::uint16_t", UInt16, 16;
    i32 => "std::int32_t", Int32, 32;
    u32 => "std::uint32_t", UInt32, 32;
    i64 => "std::int64_t", Int64, 64;
    u64 => "std::uint64_t", UInt64, 64;
}

impl Scalar for f32 {
    const TYPE_NAME: &'static str = "float";
    const COLUMN: ColumnType = ColumnType::Real32;
    const BITS: u16 = 32;
    fn page(values: &[Self]) -> Vec<u8> {
        le_bytes(values, |x| x.to_le_bytes())
    }
}

impl Scalar for f64 {
    const TYPE_NAME: &'static str = "double";
    const COLUMN: ColumnType = ColumnType::Real64;
    const BITS: u16 = 64;
    fn page(values: &[Self]) -> Vec<u8> {
        le_bytes(values, |x| x.to_le_bytes())
    }
}

impl Scalar for bool {
    const TYPE_NAME: &'static str = "bool";
    const COLUMN: ColumnType = ColumnType::Bit;
    const BITS: u16 = 1;
    fn page(values: &[Self]) -> Vec<u8> {
        pack_bits(values)
    }
}

/// A leaf field of one value per entry.
fn scalar_node<T: Scalar>(name: &str, values: &[T]) -> Node {
    let column = raw(T::COLUMN, T::BITS, T::page(values), values.len());
    leaf_node(name, T::TYPE_NAME, column)
}

/// A field made of other fields, with no column and no flags; the few
/// fields that have either set them on the result.
fn parent_node(name: &str, type_name: String, role: u16, children: Vec<Node>) -> Node {
    Node {
        name: name.to_string(),
        type_name,
        role,
        cols: vec![],
        children,
        flags: 0,
        array_size: None,
        type_checksum: None,
    }
}

/// A `float` field stored as the top `bits` bits of each value.
fn truncated_node(name: &str, values: &[f32], bits: u16) -> Node {
    let shift = 32 - u32::from(bits);
    let packed: Vec<u64> = values
        .iter()
        .map(|&x| u64::from(x.to_bits() >> shift))
        .collect();
    let page = pack_uints(&packed, bits);
    let column = raw(ColumnType::Real32Trunc, bits, page, values.len());
    leaf_node(name, "float", column)
}

/// A `float` field stored as `bits`-bit steps across `[min, max]`; values
/// outside the range are clamped to it.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "t in [0, 1]: a quantized value within its bit width"
)]
fn quantized_node(name: &str, values: &[f32], (min, max): (f64, f64), bits: u16) -> Node {
    let denom = ((1u64 << bits) - 1) as f64;
    let span = max - min;
    let packed: Vec<u64> = values
        .iter()
        .map(|&x| {
            let t = if span != 0.0 {
                ((f64::from(x) - min) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (t * denom).round() as u64
        })
        .collect();
    let page = pack_uints(&packed, bits);
    let mut column = raw(ColumnType::Real32Quant, bits, page, values.len());
    column.value_range = Some((min, max));
    leaf_node(name, "float", column)
}

/// A `float` field stored at half precision.
fn half_node(name: &str, values: &[f32]) -> Node {
    let page = le_bytes(values, |&x| f32_to_half(x).to_le_bytes());
    leaf_node(
        name,
        "float",
        raw(ColumnType::Real16, 16, page, values.len()),
    )
}

/// Lower one field's [`Column`], stored as `data_type`, into a [`Node`]
/// subtree. `class` names a struct with named members.
fn lower_column(name: &str, data: &Column, data_type: &DataType, class: &str) -> Node {
    // std::atomic<T> is a leaf field that delegates to a single child leaf
    // `_0` carrying the bare value (no column of its own).
    if let DataType::Atomic(of) = data_type {
        let child = lower_column("_0", data, of, class);
        let type_name = format!("std::atomic<{}>", child.type_name);
        return parent_node(name, type_name, ROLE_LEAF, vec![child]);
    }
    match data {
        Column::Bool(v) => scalar_node(name, v),
        Column::I8(v) => scalar_node(name, v),
        Column::U8(v) => scalar_node(name, v),
        Column::I16(v) => scalar_node(name, v),
        Column::U16(v) => scalar_node(name, v),
        Column::I32(v) => scalar_node(name, v),
        Column::U32(v) => scalar_node(name, v),
        Column::I64(v) => scalar_node(name, v),
        Column::U64(v) => scalar_node(name, v),
        Column::F64(v) => scalar_node(name, v),
        Column::F32(v) => match *data_type {
            DataType::Half => half_node(name, v),
            DataType::Truncated { bits } => truncated_node(name, v, bits),
            DataType::Quantized { bits, min, max } => quantized_node(name, v, (min, max), bits),
            _ => scalar_node(name, v),
        },
        Column::String { offsets, text } => string_node(name, offsets, text),
        Column::List { offsets, values } => {
            let child = lower_column("_0", values, &item_type(data_type, values), class);
            let ends = on_disk_offsets(offsets);
            let mut node = collection_node(name, ends, ends.len(), child);
            // A set or a map has the shape of a vector; only the name differs.
            if matches!(data_type, DataType::Set(_) | DataType::Map(..)) {
                node.type_name = data_type.cpp_name();
            }
            node
        }
        Column::Array { len, values } => match (data_type, &**values) {
            // A bitset keeps its bits in a column of its own.
            (DataType::Bitset(_), Column::Bool(bits)) => {
                let mut node = parent_node(name, format!("std::bitset<{len}>"), ROLE_LEAF, vec![]);
                node.cols = vec![raw(ColumnType::Bit, 1, pack_bits(bits), bits.len())];
                node.flags = FIELD_FLAG_ARRAY;
                node.array_size = Some(*len as u64);
                node
            }
            // A fixed array: no column of its own, one element child `_0`
            // carrying the flattened values; the field records the count.
            (_, values) => {
                let child = lower_column("_0", values, &item_type(data_type, values), class);
                let type_name = format!("std::array<{},{}>", child.type_name, len);
                let mut node = parent_node(name, type_name, ROLE_LEAF, vec![child]);
                node.flags = FIELD_FLAG_ARRAY;
                node.array_size = Some(*len as u64);
                node
            }
        },
        Column::Optional { valid, values } => {
            // optional and unique_ptr share the collection shape; only the
            // wrapper's name differs. The index column counts present entries.
            let child = lower_column("_0", values, &item_type(data_type, values), class);
            let wrapper = match data_type {
                DataType::UniquePtr(_) => "std::unique_ptr",
                _ => "std::optional",
            };
            collection_node_wrapped(name, wrapper, &present_offsets(valid), valid.len(), child)
        }
        Column::Struct(members) => {
            let types = member_types(data_type, members);
            let children: Vec<Node> = (members.iter().zip(&types))
                .map(|((n, c), t)| lower_column(n, c, t, &member_class(class, n)))
                .collect();
            if positional(members) {
                return parent_node(name, record_type_name(&children), ROLE_RECORD, children);
            }
            // A user class: ROOT checks the checksum against its dictionary.
            let mut node = parent_node(name, class.to_string(), ROLE_RECORD, children);
            node.flags = FIELD_FLAG_CHECKSUM;
            node.type_checksum = Some(class_checksum(class, members));
            node
        }
        Column::Union {
            tags,
            indices,
            alternatives,
        } => {
            // The switch column holds an 8-byte index and a 4-byte tag per
            // entry; the alternatives follow as `_0`, `_1`, ….
            let mut switch = Vec::with_capacity(tags.len() * 12);
            for (&tag, &index) in tags.iter().zip(indices) {
                switch.extend_from_slice(&index.to_le_bytes());
                switch.extend_from_slice(&tag.to_le_bytes());
            }
            let types = alternative_types(data_type, alternatives);
            let children: Vec<Node> = (alternatives.iter().zip(&types))
                .enumerate()
                .map(|(k, ((_, c), t))| lower_column(&format!("_{k}"), c, t, class))
                .collect();
            let inner: Vec<&str> = children.iter().map(|c| c.type_name.as_str()).collect();
            let type_name = format!("std::variant<{}>", inner.join(","));
            let mut node = parent_node(name, type_name, ROLE_VARIANT, children);
            node.cols = vec![raw(ColumnType::Switch, 96, switch, tags.len())];
            node
        }
        // `Column` can grow a shape; `check_fields` refuses a column whose
        // type is not its own, so one this writer does not know stops here.
        other => unreachable!("a {} column has no RNTuple form", other.type_name()),
    }
}

/// Assign field ids by a depth-first pre-order walk (parents before children,
/// matching ROOT's field/column ordering) and attach each node's columns.
fn flatten_tree(roots: Vec<Node>) -> (Vec<FieldPlan>, Vec<ColumnPlan>) {
    let mut fields = Vec::new();
    let mut cols = Vec::new();
    for node in roots {
        push_node(node, None, &mut fields, &mut cols);
    }
    (fields, cols)
}

fn push_node(
    node: Node,
    parent: Option<u32>,
    fields: &mut Vec<FieldPlan>,
    cols: &mut Vec<ColumnPlan>,
) {
    let id = u32::saturating_from(fields.len());
    fields.push(FieldPlan {
        name: node.name,
        type_name: node.type_name,
        parent_id: parent.unwrap_or(id), // a top-level field is its own parent
        role: node.role,
        flags: node.flags,
        array_size: node.array_size,
        type_checksum: node.type_checksum,
    });
    for c in node.cols {
        cols.push(ColumnPlan {
            column_type: c.column_type,
            bits: c.bits,
            field_id: id,
            page: c.page,
            n: c.n,
            value_range: c.value_range,
        });
    }
    for child in node.children {
        push_node(child, Some(id), fields, cols);
    }
}

/// Lower user fields into field and column plans (depth-first, parents before
/// children), returning the top-level entry count.
pub(super) fn lower(fields: &[Field]) -> Result<(Vec<FieldPlan>, Vec<ColumnPlan>, u32)> {
    check_fields(fields)?;
    // The entry count is a 32-bit field on disk; reject an over-large batch
    // rather than silently wrapping it into a corrupt-but-accepted file.
    let n_rows = fields
        .iter()
        .find_map(|f| entry_count(&f.data))
        .unwrap_or(0);
    let n_entries = u32::try_from(n_rows).map_err(|_| {
        Error::InvalidInput(format!(
            "RNTuple batch has {n_rows} entries, over the {} limit for one write",
            u32::MAX
        ))
    })?;
    let roots: Vec<Node> = fields
        .iter()
        .map(|f| lower_column(&f.name, &f.data, &f.data_type, &f.class))
        .collect();
    let (field_plans, columns) = flatten_tree(roots);
    Ok((field_plans, columns, n_entries))
}
