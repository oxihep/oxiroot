//! Error type shared across the core container code.

use std::fmt;

use oxiroot_compress::CompressError;

/// Convenience alias for results produced by this crate.
///
/// The error type defaults to [`Error`] but can be overridden, so
/// `Result<T, E>` still names the standard two-parameter type where this alias
/// has been glob-imported (as `oxiroot::prelude::*` does).
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors raised while reading or writing ROOT container structures.
///
/// Marked `#[non_exhaustive]`: match with a wildcard arm so new variants can be
/// added in a minor release without breaking downstream code.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A read ran past the end of the buffer.
    UnexpectedEof {
        /// Bytes the read required.
        needed: usize,
        /// Bytes still available in the buffer.
        available: usize,
    },
    /// A ROOT string field did not contain valid UTF-8.
    InvalidUtf8,
    /// The file did not start with the `"root"` magic bytes.
    BadMagic([u8; 4]),
    /// A streamed object's byte count did not match the bytes consumed.
    ByteCountMismatch {
        /// Byte count the object's header declared.
        expected: usize,
        /// Bytes actually consumed reading it.
        got: usize,
    },
    /// The bytes break the ROOT format: a truncated record, a size that runs
    /// past its end, an offset out of range, … The file is corrupt, or not what
    /// it claims to be.
    Format(String),
    /// No key, subdirectory, branch or field of that name.
    NotFound {
        /// What was looked for: `"key"`, `"subdirectory"`, `"branch"`, `"field"`,
        /// …
        what: &'static str,
        /// The name looked for. A key in a subdirectory is named by its path
        /// (`"dir/name"`).
        name: String,
    },
    /// A key holds a different class than the one asked for: reading a `TH2F` as
    /// a `Hist1D`, say.
    WrongClass {
        /// The key's name (its path, in a subdirectory); `None` for an object
        /// that has no key of its own, such as a member of a collection.
        name: Option<String>,
        /// The class the key holds.
        found: String,
        /// The class, or classes, that were asked for.
        expected: String,
    },
    /// A column (branch or field) holds another type than the one asked for:
    /// reading a `float` branch as `f64`, say. Reads never convert silently.
    TypeMismatch {
        /// The column's name.
        name: String,
        /// The type the column holds.
        found: String,
        /// The type that was asked for.
        expected: String,
    },
    /// An object's class version is one this crate cannot decode: older than
    /// the first version ROOT describes through streamer info, or newer than
    /// oxiroot knows.
    UnsupportedVersion {
        /// The class.
        class: String,
        /// Its version in the file.
        version: i32,
    },
    /// The file has no `TStreamerInfo` for a class it holds, and decoding the
    /// class needs one.
    MissingStreamerInfo {
        /// The class.
        class: String,
    },
    /// A checksum stored in the file does not match its data: the data is
    /// corrupt.
    ChecksumMismatch {
        /// What was checked, e.g. `RNTuple page`.
        what: String,
        /// The checksum of the data read.
        computed: u64,
        /// The checksum stored in the file.
        stored: u64,
    },
    /// The input is valid ROOT, but oxiroot does not read or write it yet: a
    /// column encoding, a streamer type, a set of objects `hadd` cannot merge,
    /// … The message says what.
    Unsupported(String),
    /// An argument cannot be used: an object without a name, an axis without
    /// edges, no inputs to merge, a batch over a format limit, … Nothing was
    /// written. The message says what and, where there is one, the fix.
    InvalidInput(String),
    /// A stored payload could not be decompressed. `source` says why; a codec
    /// this build cannot decode is [`CompressError::CodecUnavailable`].
    Decompress {
        /// What was being read (e.g. `key "h"`, `RNTuple page`), when known.
        context: Option<String>,
        /// The codec's error.
        source: CompressError,
    },
    /// Two objects were given the same key name in one directory (which would
    /// silently shadow on read). Name them distinctly, or write them to separate
    /// subdirectories.
    DuplicateName {
        /// The clashing key name.
        name: String,
        /// Where the clash is (e.g. `"the top directory"` or a subdirectory name).
        location: String,
    },
    /// A histogram operation was asked to combine incompatible binnings.
    BinningMismatch {
        /// Human-readable description of the mismatch.
        detail: String,
    },
    /// Inputs that must have the same length do not: a graph's `x` and `y`, a
    /// fill's values and weights, a tree's branches, an `RNTuple`'s fields, …
    /// Nothing was changed or written.
    LengthMismatch {
        /// What has the wrong length, e.g. `Graph y` or `branch "pt"`.
        what: String,
        /// The length it must have (that of the input it is paired with).
        expected: usize,
        /// Its actual length.
        found: usize,
    },
    /// Inputs that must share a schema do not: a streaming writer's batches,
    /// the trees or `RNTuples` being concatenated, or the trees of a chain.
    SchemaChanged {
        /// Human-readable description of the schema change.
        detail: String,
    },
    /// A file written in the 32-bit ("small") container form grew past the
    /// ~2 GiB it can address. Write it in the 64-bit form instead (e.g.
    /// `TreeWriter::create_large`); nothing it wrote is usable.
    FileTooLarge {
        /// The size the file reached, in bytes.
        size: u64,
    },
    /// An underlying I/O error. The [`std::io::ErrorKind`] is preserved so
    /// callers can branch on it; the message is rendered to a string so `Error`
    /// stays `Clone`.
    Io {
        /// The kind of the originating I/O error.
        kind: std::io::ErrorKind,
        /// The rendered error message.
        message: String,
    },
    /// An error from another crate (statistics, formulas, plotting,
    /// histograms, …), kept whole: [`source`](std::error::Error::source) and
    /// [`downcast_ref`](Error::downcast_ref) give it back. Build one with
    /// [`Error::other`], e.g. `.map_err(oxiroot::Error::other)?`.
    Other(External),
}

/// An error from outside this crate, held by [`Error::Other`]. Shared (so the
/// error stays `Clone`) and compared by message.
#[derive(Clone)]
pub struct External(std::sync::Arc<dyn std::error::Error + Send + Sync + 'static>);

impl fmt::Debug for External {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl PartialEq for External {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0) || self.0.to_string() == other.0.to_string()
    }
}

impl Eq for External {}

impl Error {
    /// Wrap an error from another crate (see [`Error::Other`]).
    pub fn other(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Other(External(std::sync::Arc::new(error)))
    }

    /// The wrapped error, if this is an [`Error::Other`] holding a `T`.
    #[must_use]
    pub fn downcast_ref<T: std::error::Error + 'static>(&self) -> Option<&T> {
        match self {
            Self::Other(External(e)) => e.downcast_ref::<T>(),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { name, location } => {
                write!(
                    f,
                    "duplicate key name {name:?} in {location}: two objects with the same name \
                     would shadow each other on read"
                )
            }
            Self::UnexpectedEof { needed, available } => {
                write!(
                    f,
                    "unexpected end of buffer: needed {needed} bytes, {available} available"
                )
            }
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in ROOT string"),
            Self::BadMagic(m) => {
                write!(f, "bad file magic {m:02x?} (expected \"root\")")
            }
            Self::ByteCountMismatch { expected, got } => {
                write!(
                    f,
                    "byte-count mismatch: object ends at {expected} but cursor is at {got}"
                )
            }
            Self::Format(s) => write!(f, "format error: {s}"),
            Self::NotFound { what, name } => write!(f, "no {what} named {name:?}"),
            Self::TypeMismatch {
                name,
                found,
                expected,
            } => write!(f, "column {name:?} holds {found}, not {expected}"),
            Self::WrongClass {
                name: None,
                found,
                expected,
            } => write!(f, "the object is a {found}, not a {expected}"),
            Self::WrongClass {
                name: Some(name),
                found,
                expected,
            } => write!(f, "key {name:?} is a {found}, not a {expected}"),
            Self::UnsupportedVersion { class, version } => {
                write!(f, "{class} class version {version} is not supported")
            }
            Self::MissingStreamerInfo { class } => {
                write!(f, "the file has no TStreamerInfo for {class}")
            }
            Self::ChecksumMismatch {
                what,
                computed,
                stored,
            } => write!(
                f,
                "{what} checksum mismatch: computed {computed:#018x}, stored {stored:#018x}"
            ),
            Self::Unsupported(s) | Self::InvalidInput(s) => f.write_str(s),
            Self::Decompress {
                context: None,
                source,
            } => write!(f, "decompression failed: {source}"),
            Self::Decompress {
                context: Some(context),
                source,
            } => write!(f, "decompressing {context}: {source}"),
            Self::BinningMismatch { detail } => write!(f, "binning mismatch: {detail}"),
            Self::LengthMismatch {
                what,
                expected,
                found,
            } => write!(
                f,
                "length mismatch: {what} has length {found}, expected {expected}"
            ),
            Self::SchemaChanged { detail } => write!(f, "schema changed: {detail}"),
            Self::FileTooLarge { size } => write!(
                f,
                "the file reached {size} bytes, more than the 32-bit container form can \
                 address (2 GiB); write it in the 64-bit form"
            ),
            Self::Io { message, .. } => write!(f, "I/O error: {message}"),
            Self::Other(External(e)) => fmt::Display::fmt(e, f),
        }
    }
}

impl Error {
    /// This error with `context` in front of its message, for the variants that
    /// carry a free-text message (`Format`, `Unsupported`, `InvalidInput`,
    /// `SchemaChanged`, `BinningMismatch`). The other variants are returned
    /// unchanged, so the error keeps its type.
    #[must_use]
    pub fn context(self, context: impl fmt::Display) -> Self {
        match self {
            Self::Format(m) => Self::Format(format!("{context}: {m}")),
            Self::Unsupported(m) => Self::Unsupported(format!("{context}: {m}")),
            Self::InvalidInput(m) => Self::InvalidInput(format!("{context}: {m}")),
            Self::SchemaChanged { detail } => Self::SchemaChanged {
                detail: format!("{context}: {detail}"),
            },
            Self::BinningMismatch { detail } => Self::BinningMismatch {
                detail: format!("{context}: {detail}"),
            },
            other => other,
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decompress { source, .. } => Some(source),
            Self::Other(External(e)) => Some(&**e),
            _ => None,
        }
    }
}

impl From<CompressError> for Error {
    fn from(source: CompressError) -> Self {
        Self::Decompress {
            context: None,
            source,
        }
    }
}

/// Decompress a stored payload into `len` bytes (see
/// [`oxiroot_compress::decompress`]), naming `what` was read if it fails.
///
/// # Errors
///
/// [`Error::Decompress`], naming `what`, if the payload does not decompress to `len` bytes.
pub fn decompress_payload(payload: &[u8], len: usize, what: impl fmt::Display) -> Result<Vec<u8>> {
    oxiroot_compress::decompress(payload, len).map_err(|source| Error::Decompress {
        context: Some(what.to_string()),
        source,
    })
}

/// `Ok` if an input has the length of the one it is paired with, else
/// [`Error::LengthMismatch`] naming `what` is wrong: so a constructor or a fill
/// never silently cuts an input short or pads it. `what` is formatted only for
/// the error.
///
/// # Errors
///
/// [`Error::LengthMismatch`] if `found` is not `expected`.
pub fn check_len(what: impl fmt::Display, expected: usize, found: usize) -> Result<()> {
    if found == expected {
        Ok(())
    } else {
        Err(Error::LengthMismatch {
            what: what.to_string(),
            expected,
            found,
        })
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}
