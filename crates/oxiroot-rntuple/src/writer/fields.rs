//! The fields a caller writes: [`Field`], one named [`Column`] and how it is
//! stored.

use oxiroot_io_core::{Column, DataType};

/// A named `RNTuple` field: a [`Column`] and the [`DataType`] it is stored as.
///
/// [`Field::new`] stores the column as its shape says: a list as a
/// `std::vector`, an array as a `std::array`, an optional as a
/// `std::optional`, a struct as a `std::pair` / `std::tuple` when its members
/// are named `_0`, `_1`, … and as a user class otherwise, a union as a
/// `std::variant`. [`stored_as`](Field::stored_as) picks another storage of
/// the same shape: a half-precision or quantized `f32`, a `std::set`, a
/// `std::map`, a `std::unique_ptr`, a `std::atomic`, a `std::bitset`.
///
/// ```
/// use oxiroot_io_core::DataType;
/// use oxiroot_rntuple::raw::Field;
///
/// let pt = Field::new("pt", vec![1.5_f32, 2.5]).stored_as(DataType::Half);
/// let hits = Field::new("hits", vec![vec![1_i32, 2], vec![]]);
/// # let _ = (pt, hits);
/// ```
#[derive(Debug)]
pub struct Field {
    /// Field name.
    pub(crate) name: String,
    /// Field data.
    pub(crate) data: Column,
    /// How the data is stored. Its shape is checked against the data when
    /// the field is written.
    pub(crate) data_type: DataType,
    /// The C++ class a struct with named members is written as.
    pub(crate) class: String,
}

impl Field {
    /// A field named `name` holding `data`, stored as its shape says.
    pub fn new(name: impl Into<String>, data: impl Into<Column>) -> Self {
        let name = name.into();
        let data = data.into();
        Self {
            class: identifier(&name),
            data_type: data.data_type(),
            data,
            name,
        }
    }

    /// This field stored as `data_type`, which must have the data's shape:
    /// its [`storage`](DataType::storage) is the data's own type. A type of
    /// another shape is refused when the field is written.
    #[must_use]
    pub fn stored_as(mut self, data_type: DataType) -> Self {
        self.data_type = data_type;
        self
    }

    /// The C++ class a struct with named members is written as, where
    /// [`Field::new`] names it after the field. A struct inside it is named
    /// `<class>_<member>`.
    #[must_use]
    pub fn class_name(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    /// The field's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's data.
    #[must_use]
    pub fn data(&self) -> &Column {
        &self.data
    }

    /// How the field is stored.
    #[must_use]
    pub fn data_type(&self) -> &DataType {
        &self.data_type
    }
}

/// `name` as a C++ identifier: other characters become `_`, and a leading
/// digit gets a `_` in front.
fn identifier(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// How the values inside a list, array or optional stored as `data_type` are
/// stored themselves. Where the type does not say (a type of another shape is
/// refused before anything is written), the values' own type stands in.
pub(super) fn item_type(data_type: &DataType, values: &Column) -> DataType {
    match (data_type, values) {
        // A map's entries are `{_0, _1}` structs of its key and value types.
        (DataType::Map(k, v), Column::Struct(_)) => DataType::Struct(vec![
            ("_0".into(), (**k).clone()),
            ("_1".into(), (**v).clone()),
        ]),
        (
            DataType::List(item)
            | DataType::CArray(item)
            | DataType::Set(item)
            | DataType::Array(_, item)
            | DataType::Optional(item)
            | DataType::UniquePtr(item),
            _,
        ) => (**item).clone(),
        _ => values.data_type(),
    }
}

/// How each of a struct's `members` is stored, from the struct's `data_type`.
pub(super) fn member_types(data_type: &DataType, members: &[(String, Column)]) -> Vec<DataType> {
    match data_type {
        DataType::Struct(types) if types.len() == members.len() => {
            types.iter().map(|(_, t)| t.clone()).collect()
        }
        _ => members.iter().map(|(_, c)| c.data_type()).collect(),
    }
}

/// How each of a union's `alternatives` is stored, from the union's
/// `data_type`.
pub(super) fn alternative_types(
    data_type: &DataType,
    alternatives: &[(String, Column)],
) -> Vec<DataType> {
    match data_type {
        DataType::Variant(types) if types.len() == alternatives.len() => types.clone(),
        _ => alternatives.iter().map(|(_, c)| c.data_type()).collect(),
    }
}

/// Whether a struct's members are named `_0`, `_1`, …: a `std::pair` or
/// `std::tuple`, where any other names make a user class.
pub(super) fn positional(members: &[(String, Column)]) -> bool {
    members
        .iter()
        .enumerate()
        .all(|(i, (name, _))| *name == format!("_{i}"))
}

/// The class a struct member named `member` is written as, inside a struct
/// of class `class`.
pub(super) fn member_class(class: &str, member: &str) -> String {
    identifier(&format!("{class}_{member}"))
}
