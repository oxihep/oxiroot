//! One API to read and write ROOT's two columnar formats, `TTree` and
//! `RNTuple`: a [`Batch`] of named, flat columns is what reading returns and
//! writing takes, whatever the format.
//!
//! ```no_run
//! use oxiroot_dataset::{Batch, Dataset, DatasetWriter, Format};
//! use oxiroot_io_core::FileReader;
//!
//! let file = FileReader::open("events.root")?;
//! let events = Dataset::open(&file, "events")?;          // TTree or RNTuple, detected
//!
//! let mut out = DatasetWriter::create("skim.root", "events", Format::Ntuple)?;
//! for batch in events.batches(["pt", "jet_pt"]) {          // one cluster at a time
//!     let batch = batch?;
//!     let pt: &[f32] = batch.values("pt")?;               // a slice, no copy
//!     let jets = batch.jagged::<f32>("jet_pt")?;          // offsets + flat values
//!     let keep: Vec<bool> = pt.iter().zip(jets.rows()).map(|(&p, j)| p > 25.0 && j.len() >= 2).collect();
//!     out.write(batch.filter(&keep)?)?;                   // a skim, no per-type code
//! }
//! out.finish()?;
//! # Ok::<(), oxiroot_io_core::Error>(())
//! ```
//!
//! # Columns
//!
//! A column is stored flat, the way Arrow stores it: scalars in one `Vec`,
//! strings as offsets into one byte buffer, variable-length lists as offsets
//! into one child column, fixed arrays as one child column of `N` values per
//! entry, optionals as a validity mask plus the present values. Reading a
//! `std::vector<float>` allocates two buffers per batch, not one `Vec` per
//! entry.
//!
//! | Rust | `TTree` | `RNTuple` |
//! |---|---|---|
//! | `bool`, `i8` … `u64`, `f32`, `f64` | scalar leaf | scalar field |
//! | `String` | `TLeafC` string | `std::string` |
//! | `Vec<T>` | `std::vector<T>` (reads `x[n]` too) | `std::vector<T>` |
//! | `[T; N]` | fixed array `x[N]` | `std::array<T, N>` |
//! | `Vec<Vec<T>>`, `Vec<String>`, `Vec<bool>` | `std::vector<…>` | `std::vector<…>` |
//! | `Option<T>` | — | `std::optional<T>` |
//! | struct ([`Column::Struct`]) | in a list only: split `std::vector<Struct>` | record / user class |
//!
//! Reads are strict: [`Batch::values::<f64>`](Batch::values) on a `float`
//! column is an [`Error::TypeMismatch`](oxiroot_io_core::Error::TypeMismatch),
//! never a silent conversion. A column a format cannot hold is refused when
//! the first batch is written, before anything reaches the file.
//!
//! With the `arrow` feature a batch converts to and from an Arrow
//! `RecordBatch` (`Batch::to_arrow`, `into_arrow`, `from_arrow`); with the
//! `polars` feature, to and from a Polars `DataFrame` (`Batch::to_polars`,
//! `into_polars`, `from_polars`). Every column shape converts, structs and
//! optionals (as nulls) included.
//!
//! # Stored types
//!
//! A column's shape does not say everything about how it is stored: a list
//! can be a `std::vector`, a `std::set` or a C-style `x[n]`, and an `f32` can
//! be kept in half precision. A [`Schema`] of [`DataType`]s says so.
//! [`Dataset::schema`] reports what a file holds, and a writer stores each
//! column as its own shape unless [`DatasetWriter::schema`] says otherwise:
//!
//! | Stored type | Held as | Format |
//! |---|---|---|
//! | [`DataType::Half`], [`Truncated`](DataType::Truncated), [`Quantized`](DataType::Quantized) | `f32` | `RNTuple` |
//! | [`DataType::CArray`] (`x[n]`) | list | `TTree` |
//! | [`DataType::Set`] | list | `RNTuple` |
//! | [`DataType::Map`] | list of `{_0, _1}` structs | `RNTuple` |
//! | [`DataType::Bitset`] | array of `bool` | `RNTuple` |
//! | [`DataType::UniquePtr`] | optional | `RNTuple` |
//! | [`DataType::Atomic`] | the bare value | `RNTuple` |
//!
//! [`Dataset::chain`] reads several datasets as one.
//!
//! The formats' own types (`oxiroot_tree::raw`, `oxiroot_rntuple::raw`) are
//! underneath, for a tool that works at the level of one format and for what
//! a batch does not hold: a `std::variant`, schema extension, split objects.
//! A tree's friends and index, and an `RNTuple`'s header and footer, are
//! reached through [`Dataset::as_tree`] and [`Dataset::as_ntuple`].

#[cfg(feature = "arrow")]
mod arrow_conv;
mod batch;
mod column;
#[cfg(feature = "polars")]
mod polars_conv;
mod read;
mod schema;
mod write;

pub use batch::Batch;
pub use column::{Column, IntoColumn, Jagged, Primitive, Strings};
pub use read::{Batches, Dataset, Format};
pub use schema::{DataType, Schema};
pub use write::{DatasetObject, DatasetWriter};

/// The arrow-rs crates the `arrow` feature converts to and from, re-exported
/// so their versions always match.
#[cfg(feature = "arrow")]
pub mod arrow_rs {
    pub use arrow_array as array;
    pub use arrow_schema as schema;
}

/// The Polars crate the `polars` feature converts to and from, re-exported so
/// its version always matches.
#[cfg(feature = "polars")]
pub use ::polars as polars_rs;
