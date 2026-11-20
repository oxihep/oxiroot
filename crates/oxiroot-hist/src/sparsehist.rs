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
use oxiroot_io_core::{FileReader, RBuffer, Result, read_named, read_object_base};

use crate::axis::Axis;
use crate::base::{BinContentType, check_len, object_bytes, object_bytes_in, read_tarray};

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
    pub name: String,
    /// Title (`fTitle`).
    pub title: String,
    /// One axis per dimension (`fAxes`).
    pub axes: Vec<Axis>,
    /// Number of entries (`fEntries`).
    pub entries: f64,
    /// Sum of weights (`fTsumw`).
    pub tsumw: f64,
    /// Sum of squared weights (`fTsumw2`; ROOT writes `-1` when not tracked).
    pub tsumw2: f64,
    /// Per-axis `Σ(w·x)` (`fTsumwx`).
    pub tsumwx: Vec<f64>,
    /// Per-axis `Σ(w·x²)` (`fTsumwx2`).
    pub tsumwx2: Vec<f64>,
    /// The filled cells (sparse storage). Editing it directly is allowed; a
    /// fill notices cells added or removed, but not an edit that changes one
    /// cell's coordinates to another cell's, which would duplicate that cell.
    pub bins: Vec<SparseBin>,
    /// Where each cell's coordinates sit in `bins`, so a fill finds its cell
    /// without scanning every filled one. Derived from `bins` (which is
    /// public), so it is checked on use and rebuilt when it no longer matches.
    index: CellIndex,
}

/// A lookup from cell coordinates to their position in `bins`; derived data,
/// so it never makes two histograms differ.
#[derive(Debug, Clone, Default)]
struct CellIndex(std::collections::HashMap<Vec<i32>, usize>);

impl PartialEq for CellIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl SparseHist {
    /// The position in `bins` of the cell at `coords`, if it is filled. The
    /// index is rebuilt first if `bins` was changed directly.
    fn cell(&mut self, coords: &[i32]) -> Option<usize> {
        let fresh = self.index.0.len() == self.bins.len()
            && self
                .index
                .0
                .get(coords)
                .is_none_or(|&i| self.bins.get(i).is_some_and(|b| b.coords == coords));
        if !fresh {
            self.index.0 = (self.bins.iter().enumerate())
                .map(|(i, b)| (b.coords.clone(), i))
                .collect();
        }
        self.index.0.get(coords).copied()
    }

    /// Create an empty `SparseHist` over the given per-dimension uniform axes,
    /// `(nbins, xmin, xmax)`.
    pub fn new(axes: &[(i32, f64, f64)]) -> SparseHist {
        let n = axes.len();
        SparseHist {
            name: String::new(),
            title: String::new(),
            axes: axes
                .iter()
                .enumerate()
                .map(|(i, &(nb, lo, hi))| Axis::new(&format!("axis{i}"), nb, lo, hi))
                .collect(),
            entries: 0.0,
            tsumw: 0.0,
            tsumw2: -1.0, // ROOT's "moments not tracked" marker
            tsumwx: vec![0.0; n],
            tsumwx2: vec![0.0; n],
            bins: Vec::new(),
            index: CellIndex::default(),
        }
    }

    /// Number of dimensions.
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
        let bins: Vec<i32> = self
            .axes
            .iter()
            .zip(coords)
            .map(|(ax, &x)| i32::saturating_from(ax.find_bin(x)))
            .collect();
        self.entries += 1.0;
        match self.cell(&bins) {
            Some(i) => self.bins[i].content += 1.0,
            None => {
                self.index.0.insert(bins.clone(), self.bins.len());
                self.bins.push(SparseBin {
                    coords: bins,
                    content: 1.0,
                });
            }
        }
        Ok(())
    }

    /// Per-axis bit widths used to pack a compact coordinate
    /// (`bits[d] = bitlength(nbins[d] + 2)`).
    pub(crate) fn axis_bits(&self) -> Vec<u32> {
        self.axes
            .iter()
            .map(|ax| {
                let cells = (ax.nbins.max(0) + 2).cast_unsigned();
                u32::BITS - cells.leading_zeros()
            })
            .collect()
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

    pub(crate) fn read(r: &mut RBuffer) -> Result<SparseHist> {
        let tt = r.read_version()?; // THnSparseT<TArrayD> wrapper
        let _ts = r.read_version()?; // SparseHist v3
        let thnbase = r.read_version()?; // THnBase v1

        let named = read_named(r)?;
        let ndim = usize::saturating_from(r.be_i32()?);
        let axes = read_axis_array(r)?;
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
        let bins = read_bin_content(r, ndim)?;

        if let Some(end) = tt.end {
            r.seek(end)?;
        }

        let mut h = SparseHist {
            name: named.name,
            title: named.title,
            axes,
            entries,
            tsumw,
            tsumw2,
            tsumwx,
            tsumwx2,
            bins: Vec::new(),
            index: CellIndex::default(),
        };
        let bit_widths = h.axis_bits();
        h.bins = bins
            .into_iter()
            .map(|(coord, content)| SparseBin {
                coords: h.unpack(coord, &bit_widths),
                content,
            })
            .collect();
        Ok(h)
    }
}

/// Consume an object-pointer header (`{byte count}{class tag}`), leaving the
/// cursor at the object body. Returns `false` for a null pointer.
fn enter_object(r: &mut RBuffer) -> Result<bool> {
    let bc = r.be_i32()?.cast_unsigned();
    if bc == 0 {
        return Ok(false); // null pointer
    }
    let tag = r.be_i32()?.cast_unsigned();
    if tag == 0xFFFF_FFFF {
        while r.u8()? != 0 {} // kNewClassTag: NUL-terminated class name
    }
    Ok(true)
}

/// Read a `TObjArray` header, returning the element count.
fn read_objarray_len(r: &mut RBuffer) -> Result<usize> {
    let _oa = r.read_version()?; // TObjArray version 3
    read_object_base(r)?;
    let _name = r.string()?;
    let size = usize::saturating_from(r.be_i32()?);
    let _lower_bound = r.be_i32()?;
    Ok(size)
}

/// Read `fAxes`: a `TObjArray` of `Axis` object pointers.
fn read_axis_array(r: &mut RBuffer) -> Result<Vec<Axis>> {
    let n = read_objarray_len(r)?;
    (0..n)
        .map(|_| {
            enter_object(r)?;
            Axis::read(r)
        })
        .collect()
}

/// Read `fBinContent`: a `TObjArray` of `THnSparseArrayChunk`. Returns each
/// filled bin as `(compact coordinate, content)`.
fn read_bin_content(r: &mut RBuffer, _ndim: usize) -> Result<Vec<(u64, f64)>> {
    let nchunks = read_objarray_len(r)?;
    let mut out = Vec::new();
    for _ in 0..nchunks {
        if !enter_object(r)? {
            continue;
        }
        let _chunk = r.read_version()?; // THnSparseArrayChunk v1
        read_object_base(r)?;
        let single = usize::saturating_from(r.be_i32()?); // fSingleCoordinateSize
        let coords_size = usize::saturating_from(r.be_i32()?); // fCoordinatesSize
        let _marker = r.u8()?; // char* presence flag
        let coords = r.bytes(coords_size)?.to_vec();
        // fContent: a TArrayD* of the contents (sized to the chunk capacity).
        let content = if enter_object(r)? {
            read_tarray(r, BinContentType::F64)?
        } else {
            Vec::new()
        };
        // fSumw2: a TArrayD* (often null); skip it.
        if enter_object(r)? {
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

/// Read a `SparseHist` named `name` from `file`.
pub(crate) fn read_thnsparse(file: &FileReader, name: &str) -> Result<SparseHist> {
    SparseHist::read(&mut RBuffer::new(&object_bytes(
        file,
        name,
        "THnSparseT<TArrayD>",
    )?))
}

/// Read a `THnSparseT<TArrayD>` from subdirectory `subdir`.
pub(crate) fn read_thnsparse_in(file: &FileReader, subdir: &str, name: &str) -> Result<SparseHist> {
    SparseHist::read(&mut RBuffer::new(&object_bytes_in(
        file,
        subdir,
        name,
        "THnSparseT<TArrayD>",
    )?))
}

#[cfg(test)]
mod index_tests {
    use super::*;

    /// The cell index gives the same cells and contents as counting by hand,
    /// and notices cells removed from `bins` directly.
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
        // Edit `bins` directly: drop a cell, then fill it again.
        let gone = h.bins.remove(0);
        let x: Vec<f64> = gone.coords.iter().map(|&c| f64::from(c - 1)).collect();
        h.fill(&x).unwrap();
        let again = h.bins.iter().filter(|b| b.coords == gone.coords).count();
        assert_eq!(again, 1);
    }
}
