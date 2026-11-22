//! The read → write round trip of one field: [`FieldRead::to_field`] reads a
//! field and hands it back as a [`Field`] stored the way the file stores it.
//! Merging whole `RNTuple`s is the `oxiroot` facade's `hadd`, which goes
//! through the `Dataset` reader and writer.

use oxiroot_io_core::Result;

use crate::reader::FieldRead;
use crate::writer::Field;

impl FieldRead<'_, '_> {
    /// Read the field (the [`range`](Self::range) set, if any) as a writable
    /// [`Field`] of the same name and stored type, ready for an
    /// [`NtupleWriter`](crate::raw::NtupleWriter): the round trip of a skim.
    ///
    /// # Errors
    ///
    /// Any error reading the field.
    pub fn to_field(self) -> Result<Field> {
        let name = self.name();
        let header = self.header();
        let values = self.values()?;
        let data_type = header.stored_type(name, values.data_type());
        Ok(Field::new(name, values).stored_as(data_type))
    }
}
