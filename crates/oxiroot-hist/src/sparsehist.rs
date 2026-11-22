//! `SparseHist` — an N-dimensional *sparse* histogram. Only filled bins are
//! stored: ROOT keeps them in `THnSparseArrayChunk`s, each holding the filled
//! bins' packed global coordinates (`char*`) and contents (`TArrayD`). On disk
//! the class is `THnSparseT<TArrayD>` → `SparseHist` → `THnBase`.
//!
//! The compact coordinate of a filled bin is its global linear index across all
//! axes, with `bits[d] = bitlength(nbins[d] + 2)` bits per axis (so axis `d`'s
//! stride is `2^Σ_{e<d} bits[e]`), packed big-endian into
//! `fSingleCoordinateSize = ceil(Σ bits / 8)` bytes.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::raw::{TagReader, read_named, read_object_base};
use oxiroot_io_core::{Error, RBuffer, Result};
use oxiroot_io_core::{Object, ReadRoot};
use std::collections::HashMap;

use crate::axis::Axis;
use crate::base::{BinContentType, check_len, read_tarray};

/// A filled cell: one (per-axis, flow-inclusive) bin index per dimension, and its
/// content.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseBin {
    /// Per-axis bin index (0 = underflow … `nbins+1` = overflow) for each dimension.
    pub coords: Vec<i32>,
    /// Bin content.
    pub content: f64,
}

/// An N-dimensional sparse histogram (ROOT `THnSparse`/`THnSparseT<TArrayD>`).
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "THnSparse")]
pub struct SparseHist {
    /// Name (`fName`).
    pub(crate) name: String,
    /// Title (`fTitle`).
    pub(crate) title: String,
    /// One axis per dimension (`fAxes`).
    pub(crate) axes: Vec<Axis>,
    /// Number of entries (`fEntries`).
    pub(crate) entries: f64,
    /// Sum of weights (`fTsumw`).
    pub(crate) tsumw: f64,
    /// Sum of squared weights (`fTsumw2`; ROOT writes `-1` when not tracked).
    pub(crate) tsumw2: f64,
    /// Per-axis `Σ(w·x)` (`fTsumwx`).
    pub(crate) tsumwx: Vec<f64>,
    /// Per-axis `Σ(w·x²)` (`fTsumwx2`).
    pub(crate) tsumwx2: Vec<f64>,
    /// The filled cells, in the order they were first filled. Cells are
    /// added only through [`content_mut`](Self::content_mut), which keeps
    /// `index` in step.
    pub(crate) bins: Vec<SparseBin>,
    /// Where each cell's coordinates sit in `bins`, so a fill finds its cell
    /// without scanning every filled one.
    index: HashMap<Vec<i32>, usize>,
}

impl SparseHist {
    /// The content of the cell at `coords`, to change in place. A cell not
    /// filled yet is added with a content of zero.
    pub(crate) fn content_mut(&mut self, coords: &[i32]) -> &mut f64 {
        let at = match self.index.get(coords) {
            Some(&at) => at,
            None => {
                self.index.insert(coords.to_vec(), self.bins.len());
                self.bins.push(SparseBin {
                    coords: coords.to_vec(),
                    content: 0.0,
                });
                self.bins.len() - 1
            }
        };
        &mut self.bins[at].content
    }

    /// Create an empty `SparseHist` over the given per-dimension uniform axes,
    /// `(nbins, xmin, xmax)`.
    #[must_use]
    pub fn new(axes: &[(usize, f64, f64)]) -> Self {
        let n = axes.len();
        Self {
            name: String::new(),
            title: String::new(),
            axes: axes
                .iter()
                .enumerate()
                .map(|(i, &(nb, lo, hi))| Axis::new(nb, lo, hi).named(format!("axis{i}")))
                .collect(),
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: -1.0, // ROOT's "moments not tracked" marker
            tsumwx: vec![0.0; n],
            tsumwx2: vec![0.0; n],
            bins: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Number of dimensions.
    #[must_use]
    pub fn ndim(&self) -> usize {
        self.axes.len()
    }

    /// Fill the cell containing `coords` (one value per dimension) with unit
    /// weight, accumulating into an existing filled cell or creating a new one.
    ///
    /// # Errors
    /// [`Error::LengthMismatch`](oxiroot_io_core::Error::LengthMismatch) if
    /// `coords` does not hold exactly [`ndim`](Self::ndim) values; nothing is
    /// filled.
    pub fn fill(&mut self, coords: &[f64]) -> Result<()> {
        check_len("THnSparse fill coordinates", self.ndim(), coords.len())?;
        self.check_coordinate_width()?;
        let bins: Vec<i32> = self
            .axes
            .iter()
            .zip(coords)
            .map(|(ax, &x)| i32::saturating_from(ax.find_bin(x)))
            .collect();
        self.entries += 1.0;
        *self.content_mut(&bins) += 1.0;
        Ok(())
    }

    /// A filled cell is stored under one packed coordinate, which this crate
    /// holds in 64 bits. ROOT can pack wider ones (many axes, or very finely
    /// binned ones); they are refused rather than truncated.
    fn check_coordinate_width(&self) -> Result<()> {
        let bits: u32 = self.axes.iter().map(axis_bit_width).sum();
        if bits > u64::BITS {
            return Err(Error::Unsupported(format!(
                "a THnSparse whose packed bin coordinate takes {bits} bits \
                 (oxiroot holds at most {})",
                u64::BITS
            )));
        }
        Ok(())
    }

    /// Per-axis bit widths used to pack a compact coordinate
    /// (`bits[d] = bitlength(nbins[d] + 2)`).
    pub(crate) fn axis_bits(&self) -> Vec<u32> {
        self.axes.iter().map(axis_bit_width).collect()
    }

    /// Pack per-axis bins into the compact global coordinate.
    pub(crate) fn pack(&self, bins: &[i32], bits: &[u32]) -> u64 {
        let mut coord = 0u64;
        let mut shift = 0u32;
        for (&b, &nb) in bins.iter().zip(bits) {
            coord |= u64::saturating_from(b) << shift;
            shift += nb;
        }
        coord
    }

    /// Unpack a compact global coordinate into per-axis bins.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "a masked bit field of a packed coordinate"
    )]
    fn unpack(&self, mut coord: u64, bits: &[u32]) -> Vec<i32> {
        bits.iter()
            .map(|&nb| {
                let mask = (1u64 << nb) - 1;
                let b = (coord & mask) as i32;
                coord >>= nb;
                b
            })
            .collect()
    }

    pub(crate) fn read(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Self> {
        let tt = r.read_version()?; // THnSparseT<TArrayD> wrapper
        let _ts = r.read_version()?; // SparseHist v3
        let thnbase = r.read_version()?; // THnBase v1

        let named = read_named(r)?;
        let ndim = usize::saturating_from(r.be_i32()?);
        let axes = read_axis_array(r, tags)?;
        if axes.len() != ndim {
            return Err(Error::Format(format!(
                "THnSparse of {ndim} dimensions with {} axes",
                axes.len()
            )));
        }
        let entries = r.be_f64()?;
        let tsumw = r.be_f64()?;
        let tsumw2 = r.be_f64()?;
        let tsumwx = read_tarray(r, BinContentType::F64)?;
        let tsumwx2 = read_tarray(r, BinContentType::F64)?;
        if let Some(end) = thnbase.end {
            r.seek(end)?;
        }

        let _chunk_size = r.be_i32()?;
        let _filled_bins = r.be_i64()?;
        let bins = read_bin_content(r, tags)?;

        if let Some(end) = tt.end {
            r.seek(end)?;
        }

        let mut h = Self {
            name: named.name,
            title: named.title,
            axes,
            entries,
            tsumw,
            tsumw2,
            tsumwx,
            tsumwx2,
            bins: Vec::new(),
            index: HashMap::new(),
        };
        // Before unpacking: a wider coordinate was cut to its low 64 bits.
        h.check_coordinate_width()?;
        let bit_widths = h.axis_bits();
        for (coord, content) in bins {
            let coords = h.unpack(coord, &bit_widths);
            *h.content_mut(&coords) += content;
        }
        Ok(h)
    }
}

/// The bits one axis takes in a packed coordinate: `bitlength(nbins + 2)`.
fn axis_bit_width(axis: &Axis) -> u32 {
    let cells = axis.nbins + 2;
    usize::BITS - cells.leading_zeros()
}

/// Consume an object-pointer header (`{byte count}{class tag}`), leaving the
/// cursor at the object body. Returns `false` for a null pointer.
fn enter_object(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<bool> {
    Ok(tags.read_header(r)?.class_name.is_some())
}

/// Read a `TObjArray` header, returning the element count.
fn read_objarray_len(r: &mut RBuffer<'_>) -> Result<usize> {
    let _oa = r.read_version()?; // TObjArray version 3
    read_object_base(r)?;
    r.skip_string()?;
    let size = usize::saturating_from(r.be_i32()?);
    let _lower_bound = r.be_i32()?;
    Ok(size)
}

/// Read `fAxes`: a `TObjArray` of `Axis` object pointers.
fn read_axis_array(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Vec<Axis>> {
    let n = read_objarray_len(r)?;
    (0..n)
        .map(|_| {
            enter_object(r, tags)?;
            Axis::read(r, tags)
        })
        .collect()
}

/// Read `fBinContent`: a `TObjArray` of `THnSparseArrayChunk`. Returns each
/// filled bin as `(compact coordinate, content)`.
fn read_bin_content(r: &mut RBuffer<'_>, tags: &mut TagReader) -> Result<Vec<(u64, f64)>> {
    let nchunks = read_objarray_len(r)?;
    let mut out = Vec::new();
    for _ in 0..nchunks {
        if !enter_object(r, tags)? {
            continue;
        }
        let _chunk = r.read_version()?; // THnSparseArrayChunk v1
        read_object_base(r)?;
        let single = usize::saturating_from(r.be_i32()?); // fSingleCoordinateSize
        if single > size_of::<u64>() {
            return Err(Error::Unsupported(format!(
                "a THnSparse chunk with {single}-byte packed coordinates \
                 (oxiroot holds at most {})",
                size_of::<u64>()
            )));
        }
        let coords_size = usize::saturating_from(r.be_i32()?); // fCoordinatesSize
        let _marker = r.u8()?; // char* presence flag
        let coords = r.bytes(coords_size)?.to_vec();
        // fContent: a TArrayD* of the contents (sized to the chunk capacity).
        let content = if enter_object(r, tags)? {
            read_tarray(r, BinContentType::F64)?
        } else {
            Vec::new()
        };
        // fSumw2: a TArrayD* (often null); skip it.
        if enter_object(r, tags)? {
            read_tarray(r, BinContentType::F64)?;
        }
        let n_filled = coords_size.checked_div(single).unwrap_or(0);
        for i in 0..n_filled {
            let mut c = 0u64;
            for b in &coords[i * single..(i + 1) * single] {
                c = (c << 8) | u64::from(*b); // big-endian
            }
            out.push((c, content.get(i).copied().unwrap_or(0.0)));
        }
    }
    Ok(out)
}

impl ReadRoot for SparseHist {
    const CLASS: &'static str = "THnSparseT<TArrayD>";

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| {
            let (mut r, mut tags) = object.reader()?;
            Self::read(&mut r, &mut tags)
        })
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;

    /// The cell index gives the same cells and contents as counting by hand.
    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "bin coordinates, whole numbers within the axis"
    )]
    fn indexed_fill_matches_a_plain_count() {
        let mut h = SparseHist::new(&[(50, 0.0, 50.0), (50, 0.0, 50.0), (4, 0.0, 4.0)]);
        let mut expect: std::collections::HashMap<Vec<i32>, f64> = Default::default();
        for i in 0..20_000u32 {
            let p = [
                f64::from(i * 7 % 50),
                f64::from(i * 13 % 50),
                f64::from(i % 4),
            ];
            h.fill(&p).unwrap();
            let key: Vec<i32> = p.iter().map(|&x| x as i32 + 1).collect();
            *expect.entry(key).or_default() += 1.0;
        }
        assert_eq!(h.bins.len(), expect.len());
        for b in &h.bins {
            assert_eq!(Some(&b.content), expect.get(&b.coords), "{:?}", b.coords);
        }
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    /// Seven axes of 1000 bins pack into 70 bits, more than the 64 a packed
    /// coordinate is held in: refused, where it used to be cut short.
    #[test]
    fn a_coordinate_wider_than_64_bits_is_refused() {
        let mut wide = SparseHist::new(&[(1000, 0.0, 1.0); 7]);
        let err = wide.fill(&[0.5; 7]).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
        assert!(wide.bins.is_empty(), "nothing was filled");

        let mut fits = SparseHist::new(&[(1000, 0.0, 1.0); 6]);
        fits.fill(&[0.5; 6]).unwrap();
        assert_eq!(fits.bins.len(), 1);
    }
}
