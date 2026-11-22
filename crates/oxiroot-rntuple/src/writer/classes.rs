//! The streamer classes an `RNTuple`'s record fields need, and their checksums.

use oxiroot_io_core::streamer_gen::{ClassDef, basic, streamer_info_list};
use oxiroot_io_core::{Column, DataType};

use super::fields::{Field, alternative_types, item_type, member_class, member_types, positional};
use crate::anchor::anchor_streamer_class;

/// ROOT's class checksum (`TClass::GetCheckSum`) for a flat record: fold each
/// character of the class name, then of every member's name and C++ type, into
/// `id = id * 3 + ch`. Matches `TClass::GetCheckSum()` for plain-member classes,
/// which the `RNTuple` field record stores so ROOT can validate the on-disk schema.
pub(super) fn class_checksum(class_name: &str, members: &[(String, Column)]) -> u32 {
    let fold = |id: u32, s: &str| {
        s.bytes()
            .fold(id, |acc, b| acc.wrapping_mul(3).wrapping_add(u32::from(b)))
    };
    let mut id = fold(0, class_name);
    for (name, col) in members {
        id = fold(id, name);
        id = fold(id, checksum_type_name(col));
    }
    id
}

/// How ROOT describes a class member that is a plain number: the C++ spelling
/// its class checksum folds in, and the `(fType, fSize, fTypeName)` of its
/// streamer element. `None` for any other column. An `f32` is a `float`
/// however its column packs it.
fn scalar_member(col: &Column) -> Option<(&'static str, i32, i32, &'static str)> {
    Some(match col {
        Column::Bool(_) => ("bool", 18, 1, "bool"),
        Column::I8(_) => ("char", 1, 1, "char"),
        Column::U8(_) => ("unsigned char", 11, 1, "unsigned char"),
        Column::I16(_) => ("short", 2, 2, "short"),
        Column::U16(_) => ("unsigned short", 12, 2, "unsigned short"),
        Column::I32(_) => ("int", 3, 4, "int"),
        Column::U32(_) => ("unsigned int", 13, 4, "unsigned int"),
        Column::I64(_) => ("long long", 16, 8, "Long64_t"),
        Column::U64(_) => ("unsigned long long", 17, 8, "ULong64_t"),
        Column::F32(_) => ("float", 5, 4, "float"),
        Column::F64(_) => ("double", 8, 8, "double"),
        _ => return None,
    })
}

/// The C++ type spelling ROOT uses for a member when computing a class checksum
/// (the fundamental-type keyword, e.g. `int`/`double`); `void` for a member that
/// is not a plain number, which the supported checksum does not cover.
fn checksum_type_name(col: &Column) -> &'static str {
    scalar_member(col).map_or("void", |(name, ..)| name)
}

/// Add the `TStreamerInfo` entries for the user classes inside `col`, stored
/// as `data_type` and named from `class`, to `classes`, once per class name.
/// The walk is the one the lowering makes, so the two name a struct alike. A
/// class is described when every member is a plain number, as ROOT writes it
/// for a struct without `ClassDef` (version 1); its checksum is the one the
/// field record carries.
fn collect_classes(
    col: &Column,
    data_type: &DataType,
    class: &str,
    classes: &mut Vec<ClassDef<'static>>,
) {
    if let DataType::Atomic(of) = data_type {
        return collect_classes(col, of, class, classes);
    }
    match col {
        Column::Struct(members) => {
            for ((name, member), t) in members.iter().zip(member_types(data_type, members)) {
                collect_classes(member, &t, &member_class(class, name), classes);
            }
            if positional(members) {
                return;
            }
            let elements: Option<Vec<_>> = members
                .iter()
                .map(|(name, member)| {
                    scalar_member(member)
                        .map(|(_, ty, size, type_name)| basic(name, ty, size, type_name))
                })
                .collect();
            if let Some(elements) = elements
                && !classes.iter().any(|c| c.name == class)
            {
                classes.push(
                    ClassDef {
                        name: class.into(),
                        version: 1,
                        checksum: class_checksum(class, members),
                        elements,
                    }
                    .into_owned(),
                );
            }
        }
        Column::Union { alternatives, .. } => {
            let types = alternative_types(data_type, alternatives);
            for ((_, alternative), t) in alternatives.iter().zip(types) {
                collect_classes(alternative, &t, class, classes);
            }
        }
        Column::List { values, .. }
        | Column::Array { values, .. }
        | Column::Optional { values, .. } => {
            collect_classes(values, &item_type(data_type, values), class, classes);
        }
        _ => {}
    }
}

/// The `TStreamerInfo` entries a file holding `RNTuples` with these fields needs:
/// the anchor class, which ROOT describes in every `RNTuple` file, then the user
/// classes of `fields`.
pub(super) fn ntuple_classes<'a>(
    fields: impl IntoIterator<Item = &'a Field>,
) -> Vec<ClassDef<'static>> {
    let mut classes = vec![anchor_streamer_class()];
    for field in fields {
        collect_classes(&field.data, &field.data_type, &field.class, &mut classes);
    }
    classes
}

/// [`ntuple_classes`] as a serialized `TList<TStreamerInfo>`.
pub(super) fn ntuple_streamer_info<'a>(fields: impl IntoIterator<Item = &'a Field>) -> Vec<u8> {
    streamer_info_list(&ntuple_classes(fields))
}
