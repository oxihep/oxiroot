//! One API to read and write ROOT's two columnar formats, `TTree` and
//! RNTuple: a [`Batch`] of named, flat columns is what reading returns and
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
//!     out.write(&batch.filter(&keep)?)?;                  // a skim, no per-type code
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
//! | Rust | `TTree` | RNTuple |
//! |---|---|---|
//! | `bool`, `i8` … `u64`, `f32`, `f64` | scalar leaf | scalar field |
//! | `String` | `TLeafC` string | `std::string` |
//! | `Vec<T>` | `std::vector<T>` (reads `x[n]` too) | `std::vector<T>` |
//! | `[T; N]` | fixed array `x[N]` | `std::array<T, N>` |
//! | `Vec<Vec<T>>`, `Vec<String>`, `Vec<bool>` | `std::vector<…>` | `std::vector<…>` |
//! | `Option<T>` | — | `std::optional<T>` |
//! | struct ([`Column::Struct`]) | read as member columns (`hits.x`) | record / user class |
//!
//! Reads are strict: [`Batch::values::<f64>`](Batch::values) on a `float`
//! column is an [`Error::TypeMismatch`](oxiroot_io_core::Error::TypeMismatch),
//! never a silent conversion. A column a format cannot hold is refused when
//! the first batch is written, before anything reaches the file.
//!
//! The format-specific readers and writers (`oxiroot_tree`,
//! `oxiroot_rntuple`) remain underneath for what only one format has: split
//! objects, friends and indices, RNTuple half-precision and quantized floats,
//! variants and schema extension.

mod batch;
mod column;
mod read;
mod write;

pub use batch::Batch;
pub use column::{Column, IntoColumn, Jagged, Primitive, Strings};
pub use read::{Batches, Dataset, Format};
pub use write::{DatasetObject, DatasetWriter};
