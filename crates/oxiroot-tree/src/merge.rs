//! The read → write round trip of one branch: [`BranchRead::to_branch`]
//! rebuilds a branch's kind (scalar, fixed array, jagged, `std::vector`, or
//! string) so that it writes back the way it was read. Merging whole trees is
//! the `oxiroot` facade's `hadd`, which goes through the `Dataset` reader and
//! writer.

use oxiroot_io_core::{Column, Error, Result};

use crate::reader::{BranchRead, Rewrite};
use crate::writer::Branch;

impl BranchRead<'_, '_> {
    /// Read the branch (the [`range`](Self::range) set, if any) as a writable
    /// [`Branch`] of the same name and layout — scalar, fixed array, jagged
    /// array, `std::vector` or string — ready for a
    /// [`TreeWriter`](crate::raw::TreeWriter): the round trip of a skim. Layouts
    /// this crate reads but cannot write (split objects, leaflists) are an
    /// [`Error::Unsupported`].
    ///
    /// # Errors
    ///
    /// Any error reading the branch, and [`Error::Unsupported`] as described above.
    pub fn to_branch(self) -> Result<Branch> {
        let name = self.name();
        let rewrite = self
            .tree()
            .branch_rewrite(name)
            .ok_or_else(|| Error::NotFound {
                what: "branch",
                name: name.to_string(),
            })?
            // Layouts this crate can read but not yet write back.
            .map_err(|what| {
                Error::Unsupported(format!(
                    "branch {name:?} is a {what}, which oxiroot cannot write yet",
                ))
            })?;
        build_branch(name, rewrite, self.values()?)
    }
}

/// Rebuild a writable [`Branch`] of the correct kind from a branch's read
/// metadata and its values: the column already has the branch's shape, so
/// only a counted array needs saying.
fn build_branch(name: &str, flavor: Rewrite, values: Column) -> Result<Branch> {
    let reject = |what: &str| -> Result<Branch> {
        Err(Error::Unsupported(format!(
            "branch {name:?} is a {what}, which oxiroot cannot write yet",
        )))
    };
    if let Column::List { values: inner, .. } = &values {
        match (&**inner, flavor) {
            (Column::String { .. }, _) => return reject("std::vector<std::string> branch"),
            (Column::List { .. }, _) => return reject("std::vector<std::vector<T>> branch"),
            // `std::vector<bool>` is bit-packed in memory and has no writer.
            (Column::Bool(_), Rewrite::Vector) => return reject("std::vector<bool> branch"),
            _ => {}
        }
    }
    let is_list = matches!(values, Column::List { .. });
    let branch = Branch::new(name, values);
    // Only a list has a counted form; a scalar or a string has one way.
    Ok(match (flavor, is_list) {
        (Rewrite::Jagged, true) => branch.counted(),
        _ => branch,
    })
}
