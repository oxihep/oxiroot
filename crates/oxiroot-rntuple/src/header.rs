//! The `RNTuple` header envelope: the schema (fields + columns).

use oxiroot_io_core::DataType;
use oxiroot_io_core::{RBuffer, Result};

use crate::column::ColumnType;
use crate::envelope::{read_feature_flags, read_frame, read_string};

/// Field flag: fixed-size array (a trailing `u64` array size follows).
const FIELD_FLAG_ARRAY: u16 = 0x01;
/// Field flag: projected field (a trailing `u32` source field id follows).
const FIELD_FLAG_PROJECTED: u16 = 0x02;
/// Field flag: carries a ROOT type checksum (a trailing `u32` follows).
const FIELD_FLAG_CHECKSUM: u16 = 0x04;

/// Column flag: deferred column (a trailing `i64` first-element-index follows).
const COLUMN_FLAG_DEFERRED: u16 = 0x01;
/// Column flag: ranged column (trailing `f64` min and max follow).
const COLUMN_FLAG_RANGE: u16 = 0x02;

/// Structural role of a field within the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructRole {
    /// A leaf field (scalar, or a string).
    Leaf,
    /// A collection parent (e.g. `std::vector<T>`).
    Collection,
    /// A record parent (a struct of sub-fields).
    Record,
    /// A variant parent.
    Variant,
    /// Streamed via a ROOT streamer.
    Streamer,
    /// An unrecognized role code.
    Unknown(u16),
}

impl StructRole {
    fn from_code(code: u16) -> Self {
        match code {
            0 => Self::Leaf,
            1 => Self::Collection,
            2 => Self::Record,
            3 => Self::Variant,
            4 => Self::Streamer,
            other => Self::Unknown(other),
        }
    }
}

/// A field descriptor. Its field id is its index in [`Header::fields`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// Field version.
    pub field_version: u32,
    /// Type version.
    pub type_version: u32,
    /// Parent field id (equal to own id for top-level fields).
    pub parent_field_id: u32,
    /// Structural role.
    pub struct_role: StructRole,
    /// Raw flag bits.
    pub flags: u16,
    /// Field name.
    pub name: String,
    /// C++ type name.
    pub type_name: String,
    /// Type alias.
    pub type_alias: String,
    /// Description.
    pub description: String,
    /// Fixed array size, for repetitive fields.
    pub array_size: Option<u64>,
}

/// A column descriptor. Its column index is its position in [`Header::columns`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDescriptor {
    /// Physical column type.
    pub column_type: ColumnType,
    /// Bits per stored element.
    pub bits_on_storage: u16,
    /// The field this column belongs to.
    pub field_id: u32,
    /// Raw flag bits.
    pub flags: u16,
    /// Representation index (for multi-representation columns).
    pub representation_index: u16,
    /// First element index, for deferred columns.
    pub first_element_index: Option<i64>,
    /// Value range (min, max), for ranged columns.
    pub value_range: Option<(f64, f64)>,
}

impl FieldDescriptor {
    /// The index, in [`Header::fields`], of the field this one is inside. A
    /// top-level field is its own parent.
    #[must_use]
    pub fn parent(&self) -> usize {
        self.parent_field_id as usize
    }
}

impl ColumnDescriptor {
    /// The index, in [`Header::fields`], of the field this column belongs to.
    #[must_use]
    pub fn field(&self) -> usize {
        self.field_id as usize
    }
}

/// The parsed header: the `RNTuple`'s name/description and full schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// `RNTuple` name.
    pub name: String,
    /// `RNTuple` description.
    pub description: String,
    /// Writer/library identifier.
    pub writer: String,
    /// Field descriptors (indexed by field id).
    pub fields: Vec<FieldDescriptor>,
    /// Column descriptors (indexed by column index).
    pub columns: Vec<ColumnDescriptor>,
}

impl Header {
    /// How the top-level field `name` is stored, given the `shape` its
    /// values read as: the same type with the storage choices the schema
    /// records put back in, so a half-precision float is `Half` and a
    /// `std::set` a `Set`. A field the schema does not have keeps its shape.
    #[must_use]
    pub fn stored_type(&self, name: &str, shape: DataType) -> DataType {
        match self.top_level_field(name) {
            Some(id) => field_type(self, id, shape),
            None => shape,
        }
    }

    /// The top-level fields with their index in [`fields`](Self::fields), in
    /// schema order. A field is top-level when it is its own parent.
    pub fn top_level_fields(&self) -> impl Iterator<Item = (usize, &FieldDescriptor)> + Clone {
        self.fields
            .iter()
            .enumerate()
            .filter(|(i, f)| f.parent() == *i)
    }

    /// The index of the top-level field `name`, if there is one. A field
    /// nested in another can have the same name; it is not what this finds.
    #[must_use]
    pub fn top_level_field(&self, name: &str) -> Option<usize> {
        self.top_level_fields()
            .find(|(_, f)| f.name == name)
            .map(|(i, _)| i)
    }

    /// The fields directly inside field `id`, in order.
    pub fn child_fields(&self, id: usize) -> impl Iterator<Item = usize> + '_ {
        self.fields
            .iter()
            .enumerate()
            .filter(move |&(i, f)| i != id && f.parent() == id)
            .map(|(i, _)| i)
    }

    /// Parse the header from the envelope payload (after the 8-byte type/length
    /// word, before the checksum).
    ///
    /// # Errors
    ///
    /// If `payload` is not a well-formed header envelope.
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut r = RBuffer::new(payload);

        read_feature_flags(&mut r)?;
        let name = read_string(&mut r)?;
        let description = read_string(&mut r)?;
        let writer = read_string(&mut r)?;

        let fields = read_field_list(&mut r)?;
        let columns = read_column_list(&mut r)?;
        // (Alias columns and extra type info follow; not needed yet.)

        Ok(Self {
            name,
            description,
            writer,
            fields,
            columns,
        })
    }
}

pub(crate) fn read_field_list(r: &mut RBuffer<'_>) -> Result<Vec<FieldDescriptor>> {
    let list = read_frame(r)?;
    let mut fields = Vec::with_capacity((list.n_items as usize).min(r.remaining()));
    for _ in 0..list.n_items {
        let frame = read_frame(r)?;

        let field_version = r.le_u32()?;
        let type_version = r.le_u32()?;
        let parent_field_id = r.le_u32()?;
        let struct_role = StructRole::from_code(r.le_u16()?);
        let flags = r.le_u16()?;
        let name = read_string(r)?;
        let type_name = read_string(r)?;
        let type_alias = read_string(r)?;
        let description = read_string(r)?;

        let array_size = if flags & FIELD_FLAG_ARRAY != 0 {
            Some(r.le_u64()?)
        } else {
            None
        };
        if flags & FIELD_FLAG_PROJECTED != 0 {
            let _source_field_id = r.le_u32()?;
        }
        if flags & FIELD_FLAG_CHECKSUM != 0 {
            let _type_checksum = r.le_u32()?;
        }

        r.seek(frame.end)?;
        fields.push(FieldDescriptor {
            field_version,
            type_version,
            parent_field_id,
            struct_role,
            flags,
            name,
            type_name,
            type_alias,
            description,
            array_size,
        });
    }
    r.seek(list.end)?;
    Ok(fields)
}

pub(crate) fn read_column_list(r: &mut RBuffer<'_>) -> Result<Vec<ColumnDescriptor>> {
    let list = read_frame(r)?;
    let mut columns = Vec::with_capacity((list.n_items as usize).min(r.remaining()));
    for _ in 0..list.n_items {
        let frame = read_frame(r)?;

        let column_type = ColumnType::from_code(r.le_u16()?)?;
        let bits_on_storage = r.le_u16()?;
        let field_id = r.le_u32()?;
        let flags = r.le_u16()?;
        let representation_index = r.le_u16()?;

        let first_element_index = if flags & COLUMN_FLAG_DEFERRED != 0 {
            Some(r.le_i64()?)
        } else {
            None
        };
        let value_range = if flags & COLUMN_FLAG_RANGE != 0 {
            Some((r.le_f64()?, r.le_f64()?))
        } else {
            None
        };

        r.seek(frame.end)?;
        columns.push(ColumnDescriptor {
            column_type,
            bits_on_storage,
            field_id,
            flags,
            representation_index,
            first_element_index,
            value_range,
        });
    }
    r.seek(list.end)?;
    Ok(columns)
}

/// Whether the C++ type `name` is the template `template` (`std::set`) or its
/// unordered or multi form.
fn is_template(name: &str, template: &str) -> bool {
    let base = name.split('<').next().unwrap_or(name);
    let base = base.strip_prefix("std::").unwrap_or(base);
    [
        template.to_string(),
        format!("unordered_{template}"),
        format!("multi{template}"),
        format!("unordered_multi{template}"),
    ]
    .iter()
    .any(|t| t == base)
}

/// Refine `shape` with how field `id` and the fields inside it are stored.
/// Where the header and the shape do not line up, the shape is kept as it is.
fn field_type(header: &Header, id: usize, shape: DataType) -> DataType {
    let field = &header.fields[id];
    let kids: Vec<usize> = header.child_fields(id).collect();
    let type_name = field.type_name.as_str();
    let inner = |kid: Option<&usize>, shape: DataType| match kid {
        Some(&kid) => field_type(header, kid, shape),
        None => shape,
    };
    if is_template(type_name, "atomic") && kids.len() == 1 {
        return DataType::Atomic(Box::new(field_type(header, kids[0], shape)));
    }
    match shape {
        DataType::F32 => {
            let column = header.columns.iter().find(|c| c.field() == id);
            match column.map(|c| (c.column_type, c.bits_on_storage, c.value_range)) {
                Some((ColumnType::Real16 | ColumnType::SplitReal16, ..)) => DataType::Half,
                Some((ColumnType::Real32Trunc, bits, _)) => DataType::Truncated { bits },
                Some((ColumnType::Real32Quant, bits, Some((min, max)))) => {
                    DataType::Quantized { bits, min, max }
                }
                _ => DataType::F32,
            }
        }
        DataType::List(item) if is_template(type_name, "set") => {
            DataType::Set(Box::new(inner(kids.first(), *item)))
        }
        DataType::List(item) if is_template(type_name, "map") => {
            // A map is a collection of `std::pair<K, V>` records.
            let pair: Option<Vec<usize>> = kids.first().map(|&k| header.child_fields(k).collect());
            match (*item, pair.as_deref()) {
                (DataType::Struct(members), Some([key, value])) if members.len() == 2 => {
                    let mut members = members.into_iter().map(|(_, t)| t);
                    let (k, v) = (members.next(), members.next());
                    match (k, v) {
                        (Some(k), Some(v)) => DataType::Map(
                            Box::new(field_type(header, *key, k)),
                            Box::new(field_type(header, *value, v)),
                        ),
                        _ => unreachable!("two members were counted"),
                    }
                }
                (item, _) => DataType::List(Box::new(item)),
            }
        }
        DataType::List(item) => DataType::List(Box::new(inner(kids.first(), *item))),
        DataType::Array(len, item)
            if type_name.starts_with("std::bitset<") && *item == DataType::Bool =>
        {
            DataType::Bitset(len)
        }
        DataType::Array(len, item) => DataType::Array(len, Box::new(inner(kids.first(), *item))),
        DataType::Optional(item) if type_name.starts_with("std::unique_ptr<") => {
            DataType::UniquePtr(Box::new(inner(kids.first(), *item)))
        }
        DataType::Optional(item) => DataType::Optional(Box::new(inner(kids.first(), *item))),
        DataType::Struct(members) if members.len() == kids.len() => DataType::Struct(
            members
                .into_iter()
                .zip(&kids)
                .map(|((name, t), &kid)| (name, field_type(header, kid, t)))
                .collect(),
        ),
        DataType::Variant(alternatives) if alternatives.len() == kids.len() => DataType::Variant(
            alternatives
                .into_iter()
                .zip(&kids)
                .map(|(t, &kid)| field_type(header, kid, t))
                .collect(),
        ),
        other => other,
    }
}
