//! [`Compression`]: the algorithm and level a writer applies, as ROOT's
//! `algorithm * 100 + level` setting integer.

use crate::Algorithm;

/// How a writer should compress object payloads and `RNTuple` pages.
///
/// Maps to ROOT's `algorithm * 100 + level` setting integer
/// ([`setting`](Compression::setting)), which files record in their header and
/// which [`compress`](crate::compress) takes. This crate encodes and decodes all
/// four algorithms. Levels run from 1 to 9 in ROOT; a level above 99 is treated
/// as 99, since it would otherwise spill into the algorithm digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// Store uncompressed.
    Stored,
    /// Zstandard (ROOT's default algorithm, at level 5). The pure-Rust encoder
    /// has four levels: ROOT's 1 to 3 select the first three, and 4 and above
    /// the fourth, which compresses about as well as C zstd's level 5.
    Zstd(u8),
    /// zlib / DEFLATE (ROOT's classic default, at level 1). The level tunes the
    /// encoder.
    Zlib(u8),
    /// LZ4. The pure-Rust encoder is fast-only, so the level is recorded but does
    /// not change the ratio.
    Lz4(u8),
    /// LZMA, as an XZ stream. The level selects the XZ preset.
    Lzma(u8),
}

/// What every writer in the workspace uses unless told otherwise: Zstandard at
/// level 5, ROOT's own default.
impl Default for Compression {
    fn default() -> Self {
        Self::Zstd(5)
    }
}

/// The compression a ROOT setting integer (`algorithm * 100 + level`)
/// describes. `0`, and any setting whose algorithm digit is 0, is
/// [`Compression::Stored`]; the error is the setting itself, for an algorithm
/// this crate cannot encode.
impl TryFrom<u32> for Compression {
    type Error = u32;

    fn try_from(setting: u32) -> Result<Self, u32> {
        // Below 100, so it fits.
        let level = (setting % 100) as u8;
        Ok(match setting / 100 {
            0 => Self::Stored,
            1 => Self::Zlib(level),
            2 => Self::Lzma(level),
            4 => Self::Lz4(level),
            5 => Self::Zstd(level),
            _ => return Err(setting),
        })
    }
}

impl Compression {
    /// The ROOT setting integer (`algorithm * 100 + level`, 0 = none).
    #[must_use]
    pub const fn setting(self) -> u32 {
        let (algorithm, level) = match self {
            Self::Stored => return 0,
            Self::Zlib(level) => (1, level),
            Self::Lzma(level) => (2, level),
            Self::Lz4(level) => (4, level),
            Self::Zstd(level) => (5, level),
        };
        algorithm * 100 + if level > 99 { 99 } else { level as u32 }
    }

    /// The block algorithm this setting encodes with, or `None` for
    /// [`Compression::Stored`].
    #[must_use]
    pub const fn algorithm(self) -> Option<Algorithm> {
        match self {
            Self::Stored => None,
            Self::Zstd(_) => Some(Algorithm::Zstd),
            Self::Zlib(_) => Some(Algorithm::Zlib),
            Self::Lz4(_) => Some(Algorithm::Lz4),
            Self::Lzma(_) => Some(Algorithm::Lzma),
        }
    }

    /// The level, or `None` for [`Compression::Stored`].
    #[must_use]
    pub const fn level(self) -> Option<u8> {
        match self {
            Self::Stored => None,
            Self::Zstd(level) | Self::Zlib(level) | Self::Lz4(level) | Self::Lzma(level) => {
                Some(level)
            }
        }
    }

    /// Whether anything is compressed (i.e. not [`Compression::Stored`]).
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Stored)
    }
}

#[cfg(test)]
mod tests {
    use super::Compression;

    #[test]
    fn settings_round_trip() {
        for c in [
            Compression::Stored,
            Compression::Zstd(5),
            Compression::Zlib(1),
            Compression::Lz4(4),
            Compression::Lzma(9),
        ] {
            assert_eq!(Compression::try_from(c.setting()), Ok(c));
        }
        assert_eq!(Compression::Zstd(5).setting(), 505);
        assert_eq!(
            Compression::try_from(301),
            Err(301),
            "the legacy algorithm 3"
        );
    }

    #[test]
    fn an_out_of_range_level_keeps_its_algorithm() {
        assert_eq!(Compression::Zlib(250).setting(), 199);
        assert_eq!(Compression::Zstd(u8::MAX).setting(), 599);
    }
}

#[cfg(test)]
mod level_tests {
    use super::Compression;

    #[test]
    fn level_is_none_only_for_stored() {
        assert_eq!(Compression::Stored.level(), None);
        assert_eq!(Compression::Zstd(5).level(), Some(5));
        assert_eq!(Compression::Zlib(1).level(), Some(1));
        assert_eq!(Compression::Lz4(4).level(), Some(4));
        assert_eq!(Compression::Lzma(9).level(), Some(9));
    }
}
