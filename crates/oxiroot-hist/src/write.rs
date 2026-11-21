//! Serializing a `TH1D` to ROOT's on-disk object layout.
//!
//! Reproduces the exact byte layout ROOT writes (validated by byte-comparison
//! against a ROOT-written fixture), filling the data-bearing members from a
//! [`Hist1D`] and the cosmetic/auxiliary members with ROOT's defaults.

use oxiroot_io_core::cast::SaturatingFrom;
use std::sync::LazyLock;

pub(crate) use oxiroot_io_core::WriteRoot;
pub(crate) use oxiroot_io_core::raw::write_object_ptr;
use oxiroot_io_core::raw::{K_BYTE_COUNT_MASK, write_named, write_object_base};
use oxiroot_io_core::streamer_gen::{ClassDef, StreamerInfoList};
use oxiroot_io_core::{ObjString, WBuffer};

use crate::axis::Axis;
use crate::base::BinContentType;
use crate::efficiency::Efficiency;
use crate::graph::{Graph, GraphErrors, GraphFunction};
use crate::graph2d::Graph2D;
use crate::hist1d::Hist1D;
use crate::hist2d::Hist2D;
use crate::hist3d::Hist3D;
use crate::multierrorgraph::MultiErrorGraph;
use crate::polyhist::{PolyBin, PolyHist};
use crate::profile1d::Profile1D;
use crate::profile2d::Profile2D;
use crate::profile3d::Profile3D;
use crate::sparsehist::SparseHist;

/// The streamer info ROOT 6 writes for the histogram family, captured from a
/// ROOT-written file (`scripts/gen_hist_streamers.cpp`): the histogram,
/// profile, efficiency, sparse and graph classes this crate writes, with their
/// bases and members' classes — including the `THnSparseArrayChunk` and
/// `TArrayD` a filled sparse histogram holds — plus `Func1D`, `TFormula` and
/// `TF1Parameters` (a graph's attached functions). `Func2D`/`Func3D` are not in it;
/// `oxiroot-hist-func` generates those.
static HIST_INFO: LazyLock<StreamerInfoList> = LazyLock::new(|| {
    StreamerInfoList::parse(HIST_STREAMER_INFO)
        .expect("the captured histogram streamer info parses")
});

/// The captured descriptions of `classes` and of every class they depend on,
/// for [`WriteRoot::streamer_classes`]. A histogram-family type asks for its
/// own class, plus any class it holds behind a base-class pointer or in a list
/// (an `Efficiency`'s `TH1D`s, say), so a file describes only the classes it
/// holds.
#[must_use]
pub fn hist_streamer_classes(classes: &[&str]) -> Vec<ClassDef<'static>> {
    HIST_INFO.classes_for(classes)
}

/// `ObjString`'s description, when any of `axes` carries bin labels: they are
/// stored as a `THashList` of `ObjString`s, and the captured list does not
/// describe `ObjString`.
fn label_classes<'a>(axes: impl IntoIterator<Item = &'a Axis>) -> Vec<ClassDef<'static>> {
    if axes.into_iter().any(|a| !a.labels.is_empty()) {
        ObjString::new("").streamer_classes()
    } else {
        Vec::new()
    }
}

/// `Hist1D`/`Hist2D`/`Hist3D` serialize with the bin content type carried by their `class_name`;
/// the macro picks the right `write_th{1,2,3}{d,f,i,s,c,l}` for the suffix.
macro_rules! impl_write_root_hist {
    ($ty:ty, $write:ident, $functions:expr) => {
        impl WriteRoot for $ty {
            fn root_class(&self) -> String {
                self.class_name()
            }
            fn root_name(&self) -> &str {
                &self.name
            }
            fn root_title(&self) -> &str {
                &self.title
            }
            fn to_root_bytes(&self) -> Vec<u8> {
                let mut w = WBuffer::new();
                $write(&mut w, self, self.bin_content_type);
                w.into_vec()
            }
            fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
                let class = self.class_name();
                // Attached functions are `TF1`s in the `fFunctions` list.
                let has_functions: fn(&$ty) -> bool = $functions;
                let function_class = has_functions(self).then_some("TF1");
                let names: Vec<&str> = std::iter::once(class.as_str())
                    .chain(function_class)
                    .collect();
                let mut classes = hist_streamer_classes(&names);
                classes.extend(label_classes([&self.xaxis, &self.yaxis, &self.zaxis]));
                classes
            }
        }
    };
}
impl_write_root_hist!(Hist1D, write_th1_obj, |h| !h.functions.is_empty());
impl_write_root_hist!(Hist2D, write_th2_obj, |_| false);
impl_write_root_hist!(Hist3D, write_th3_obj, |_| false);

/// A fixed-class writable type (profiles, efficiency, sparse, poly):
/// `root_class` is a constant and `to_root_bytes` delegates to its serializer.
/// The streamer info covers the class, the classes of the objects it holds that
/// its members' types do not name (`$holds`), and the bin labels of its axes
/// (`$axis`).
macro_rules! impl_write_root_fixed {
    ($ty:ty, $class:expr, $bytes:ident, [$($holds:expr),*], [$($axis:ident),*]) => {
        impl WriteRoot for $ty {
            fn root_class(&self) -> String {
                $class.to_string()
            }
            fn root_name(&self) -> &str {
                &self.name
            }
            fn root_title(&self) -> &str {
                &self.title
            }
            fn to_root_bytes(&self) -> Vec<u8> {
                $bytes(self)
            }
            fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
                let mut classes = hist_streamer_classes(&[$class $(, $holds)*]);
                classes.extend(label_classes([$(&self.$axis),*]));
                classes
            }
        }
    };
}
impl_write_root_fixed!(Profile1D, "TProfile", tprofile_to_bytes, [], [xaxis]);
impl_write_root_fixed!(
    Profile2D,
    "TProfile2D",
    tprofile2d_to_bytes,
    [],
    [xaxis, yaxis]
);
impl_write_root_fixed!(
    Profile3D,
    "TProfile3D",
    tprofile3d_to_bytes,
    [],
    [xaxis, yaxis, zaxis]
);
// Its passed and total histograms, held as `Hist1D*`.
impl_write_root_fixed!(
    Efficiency,
    "TEfficiency",
    tefficiency_to_bytes,
    ["TH1D"],
    []
);
// Its axes and its chunks, held in `TObjArray`s.
impl_write_root_fixed!(
    SparseHist,
    "THnSparseT<TArrayD>",
    thnsparse_to_bytes,
    ["TAxis", "THnSparseArrayChunk"],
    []
);
// Its bins, held in a `TList`, each with its polygon as a `TObject*`.
impl_write_root_fixed!(
    PolyHist,
    "TH2Poly",
    th2poly_to_bytes,
    ["TH2PolyBin", "TGraph"],
    []
);

impl WriteRoot for Graph {
    fn root_class(&self) -> String {
        self.class_name().to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        &self.title
    }
    fn to_root_bytes(&self) -> Vec<u8> {
        tgraph_to_bytes(self)
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        // Attached functions are `Func1D`s in the `fFunctions` list.
        let functions = if self.functions.is_empty() {
            None
        } else {
            Some("TF1")
        };
        let classes: Vec<&str> = std::iter::once(self.class_name())
            .chain(functions)
            .collect();
        hist_streamer_classes(&classes)
    }
}

/// Streamer info (`TList<TStreamerInfo>`) describing the writable histogram
/// hierarchy — `Hist1D/2/3{C,S,I,L,F,D}`, `Profile1D`, and every base/member class —
/// at the exact class versions this module emits (the `L` types are version 0).
/// Embedded in every written file so it is self-describing. Sourced from a
/// ROOT-written file with one of each type, kept uncompressed.
const HIST_STREAMER_INFO: &[u8] = include_bytes!("histograms.streamerinfo.bin");

// `fBits` values ROOT writes for the embedded TObjects in a fresh histogram.
const HIST_BITS: u32 = 0x0300_0008;
const AXIS_BITS: u32 = 0x0300_0000;
const TLIST_BITS: u32 = 0x0301_0000;

/// Serialize a `TH1{D,F,C,S,I,L}` object (with its byte-count/version header)
/// into `w`, byte-for-byte as ROOT writes it. `contents` is the type of its
/// data `TArray` base (`TArray{D,F,C,S,I,L64}`). Everything else in the object
/// is the same for every bin content type; only the outer class version
/// differs.
pub(crate) fn write_th1_obj(w: &mut WBuffer, h: &Hist1D, contents: BinContentType) {
    // The class version: 0 for the `Long64_t` classes, which came later.
    let version = if contents == BinContentType::I64 {
        0
    } else {
        3
    };
    // The fields are private, so these hold; a broken one would write a file
    // ROOT (and this crate's reader) rejects.
    debug_assert_eq!(
        h.contents.len(),
        h.expected_cells(),
        "Hist1D cells and binning disagree"
    );
    debug_assert!(
        h.sumw2.is_empty() || h.sumw2.len() == h.contents.len(),
        "Hist1D sumw2 and contents disagree"
    );
    let outer = w.begin_object(version);
    write_th1_base(w, h);
    write_tarray(w, &h.contents, contents); // TArray{…} base: bin contents, inline
    w.end_object(outer);
}

/// Serialize a `TH2D`/`TH2F` object (with its byte-count/version header) into
/// `w`, byte-for-byte as ROOT writes it. Layout: `Hist2D{D,F}{ Hist2D{ Hist1D{…},
/// fScalefactor, fTsumwy, fTsumwy2, fTsumwxy }, TArray{D,F} }`.
pub(crate) fn write_th2_obj(w: &mut WBuffer, h: &Hist2D, contents: BinContentType) {
    // The class version: 0 for the `Long64_t` classes, which came later.
    let version = if contents == BinContentType::I64 {
        0
    } else {
        4
    };
    // The fields are private, so these hold; a broken one would write a file
    // ROOT (and this crate's reader) rejects.
    debug_assert_eq!(
        h.contents.len(),
        h.expected_cells(),
        "Hist2D cells and binning disagree"
    );
    debug_assert!(
        h.sumw2.is_empty() || h.sumw2.len() == h.contents.len(),
        "Hist2D sumw2 and contents disagree"
    );
    let outer = w.begin_object(version); // Hist2D{D,F,C,S,I}=4, TH2L=0
    let th2 = w.begin_object(5); // Hist2D version 5
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &h.zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumw2,
            functions: &[],
        },
    );
    w.be_f64(1.0); // fScalefactor (ROOT default)
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    w.be_f64(h.tsumwxy);
    w.end_object(th2);
    write_tarray(w, &h.contents, contents); // TArray{D,F} base: bin contents, inline
    w.end_object(outer);
}

/// Serialize a `TH3D`/`TH3F` object (with its byte-count/version header) into
/// `w`, byte-for-byte as ROOT writes it. Layout: `Hist3D{D,F}{ Hist3D{ Hist1D{…}, TAtt3D,
/// fTsumwy, fTsumwy2, fTsumwxy, fTsumwz, fTsumwz2, fTsumwxz, fTsumwyz },
/// TArray{D,F} }`.
pub(crate) fn write_th3_obj(w: &mut WBuffer, h: &Hist3D, contents: BinContentType) {
    // The class version: 0 for the `Long64_t` classes, which came later.
    let version = if contents == BinContentType::I64 {
        0
    } else {
        4
    };
    // The fields are private, so these hold; a broken one would write a file
    // ROOT (and this crate's reader) rejects.
    debug_assert_eq!(
        h.contents.len(),
        h.expected_cells(),
        "Hist3D cells and binning disagree"
    );
    debug_assert!(
        h.sumw2.is_empty() || h.sumw2.len() == h.contents.len(),
        "Hist3D sumw2 and contents disagree"
    );
    let outer = w.begin_object(version); // Hist3D{D,F,C,S,I}=4, TH3L=0
    let th3 = w.begin_object(6); // Hist3D version 6
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &h.zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumw2,
            functions: &[],
        },
    );
    let att3d = w.begin_object(1); // TAtt3D version 1 (empty base)
    w.end_object(att3d);
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    w.be_f64(h.tsumwxy);
    w.be_f64(h.tsumwz);
    w.be_f64(h.tsumwz2);
    w.be_f64(h.tsumwxz);
    w.be_f64(h.tsumwyz);
    w.end_object(th3);
    write_tarray(w, &h.contents, contents); // TArray{D,F} base: bin contents, inline
    w.end_object(outer);
}

/// Serialize a `Profile1D` object (including its leading byte-count/version
/// header) into `w`. Layout: `Profile1D{ TH1D{ Hist1D{…, fSumw2=Σwy²}, TArrayD=Σwy },
/// fBinEntries, fErrorMode, fYmin, fYmax, fTsumwy, fTsumwy2, fBinSumw2 }`.
pub(crate) fn write_tprofile(w: &mut WBuffer, h: &Profile1D) {
    // A 1-D profile keeps degenerate y/z axes, as ROOT's Hist1D constructor does.
    let yaxis = Axis::new("yaxis", 1, 0.0, 1.0);
    let zaxis = Axis::new("zaxis", 1, 0.0, 1.0);

    let tp = w.begin_object(7); // Profile1D version 7
    let th1d = w.begin_object(3); // TH1D version 3
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &yaxis,
            zaxis: &zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumy2,
            functions: &[],
        },
    );
    write_tarrayd(w, &h.sums); // TH1D TArrayD base: per-bin sum of w*y
    w.end_object(th1d);
    write_tarrayd(w, &h.bin_entries); // fBinEntries
    w.be_i32(h.error_mode.code());
    w.be_f64(h.ymin);
    w.be_f64(h.ymax);
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    write_tarrayd(w, &h.bin_sumw2); // fBinSumw2
    w.end_object(tp);
}

/// Serialize a `Profile1D` object to a fresh byte vector.
pub(crate) fn tprofile_to_bytes(h: &Profile1D) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tprofile(&mut w, h);
    w.into_vec()
}

/// Serialize a `Profile2D` object (with its byte-count/version header) into `w`.
/// Layout: `Profile2D{ TH2D{ Hist2D{ Hist1D{…, fSumw2=Σwz²}, fScalefactor, fTsumwy,
/// fTsumwy2, fTsumwxy }, TArrayD=Σwz }, fBinEntries, fErrorMode, fZmin, fZmax,
/// fTsumwz, fTsumwz2, fBinSumw2 }`.
pub(crate) fn write_tprofile2d(w: &mut WBuffer, h: &Profile2D) {
    // A 2-D profile keeps a degenerate z axis, as ROOT's Hist2D constructor does.
    let zaxis = Axis::new("zaxis", 1, 0.0, 1.0);

    let tp = w.begin_object(8); // Profile2D version 8
    let th2d = w.begin_object(4); // TH2D version 4
    let th2 = w.begin_object(5); // Hist2D version 5
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumz2,
            functions: &[],
        },
    );
    w.be_f64(1.0); // fScalefactor (ROOT default)
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    w.be_f64(h.tsumwxy);
    w.end_object(th2);
    write_tarrayd(w, &h.sums); // TH2D TArrayD base: per-cell Σ(w·z)
    w.end_object(th2d);
    write_tarrayd(w, &h.bin_entries); // fBinEntries
    w.be_i32(h.error_mode.code());
    w.be_f64(h.zmin);
    w.be_f64(h.zmax);
    w.be_f64(h.tsumwz);
    w.be_f64(h.tsumwz2);
    write_tarrayd(w, &h.bin_sumw2); // fBinSumw2
    w.end_object(tp);
}

/// Serialize a `Profile2D` object to a fresh byte vector.
pub(crate) fn tprofile2d_to_bytes(h: &Profile2D) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tprofile2d(&mut w, h);
    w.into_vec()
}

/// Serialize a `Profile3D` object (with its byte-count/version header) into `w`.
/// Layout: `Profile3D{ TH3D{ Hist3D{ Hist1D{…, fSumw2=Σwt²}, TAtt3D, fTsumwy…fTsumwyz },
/// TArrayD=Σwt }, fBinEntries, fErrorMode, fTmin, fTmax, fTsumwt, fTsumwt2,
/// fBinSumw2 }`.
pub(crate) fn write_tprofile3d(w: &mut WBuffer, h: &Profile3D) {
    let tp = w.begin_object(8); // Profile3D version 8
    let th3d = w.begin_object(4); // TH3D version 4
    let th3 = w.begin_object(6); // Hist3D version 6
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &h.zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumt2,
            functions: &[],
        },
    );
    let att3d = w.begin_object(1); // TAtt3D version 1 (empty base)
    w.end_object(att3d);
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    w.be_f64(h.tsumwxy);
    w.be_f64(h.tsumwz);
    w.be_f64(h.tsumwz2);
    w.be_f64(h.tsumwxz);
    w.be_f64(h.tsumwyz);
    w.end_object(th3);
    write_tarrayd(w, &h.sums); // TH3D TArrayD base: per-cell Σ(w·t)
    w.end_object(th3d);
    write_tarrayd(w, &h.bin_entries); // fBinEntries
    w.be_i32(h.error_mode.code());
    w.be_f64(h.tmin);
    w.be_f64(h.tmax);
    w.be_f64(h.tsumwt);
    w.be_f64(h.tsumwt2);
    write_tarrayd(w, &h.bin_sumw2); // fBinSumw2
    w.end_object(tp);
}

/// Serialize a `Profile3D` object to a fresh byte vector.
pub(crate) fn tprofile3d_to_bytes(h: &Profile3D) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tprofile3d(&mut w, h);
    w.into_vec()
}

/// An empty `fBeta_bin_params` (`vector<pair<double,double>>`), byte-for-byte as
/// ROOT writes it (its memberwise framing is awkward to synthesize; per-bin beta
/// parameters are rarely set, so the empty form is baked).
const EFF_BETA_BIN_PARAMS_EMPTY: [u8; 16] = [
    0x40, 0x00, 0x00, 0x0c, 0x40, 0x0a, 0x00, 0x00, 0x00, 0xd7, 0xbe, 0xd2, 0x00, 0x00, 0x00, 0x00,
];

/// An empty `fFunctions` (`TList*`), byte-for-byte as ROOT writes it.
const EFF_FUNCTIONS_EMPTY: [u8; 21] = [
    0x40, 0x00, 0x00, 0x11, 0x00, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Write an embedded `Hist1D*` object pointer: `{byte count}{kNewClassTag}{"TH1D\0"}
/// {TH1D object}`. ROOT shares the class via a back-reference for the second
/// histogram; a fresh `kNewClassTag` for both is read identically.
fn write_th1d_ptr(w: &mut WBuffer, h: &Hist1D) {
    write_object_ptr(w, "TH1D", |w| write_th1_obj(w, h, BinContentType::F64));
}

/// Write an embedded `TH1F*` object pointer (a graph's `fHistogram` display
/// frame): `{byte count}{kNewClassTag}{"TH1F"}{TH1F object}`.
fn write_th1f_ptr(w: &mut WBuffer, h: &Hist1D) {
    write_object_ptr(w, "TH1F", |w| write_th1_obj(w, h, BinContentType::F32));
}

/// Serialize an `Efficiency` object (with its byte-count/version header) into `w`.
/// Layout: `Efficiency{ Named, TAttLine, TAttFill, TAttMarker, fBeta_alpha,
/// fBeta_beta, fBeta_bin_params, fConfLevel, fFunctions, fPassedHistogram(TH1D*),
/// fStatisticOption, fTotalHistogram(TH1D*), fWeight }`.
pub(crate) fn write_tefficiency(w: &mut WBuffer, h: &Efficiency) {
    let te = w.begin_object(2); // Efficiency version 2
    write_named(w, HIST_BITS, &h.name, &h.title);
    write_attline(w);
    write_attfill(w);
    write_attmarker(w);
    w.be_f64(h.beta_alpha);
    w.be_f64(h.beta_beta);
    w.bytes(&EFF_BETA_BIN_PARAMS_EMPTY);
    w.be_f64(h.conf_level);
    w.bytes(&EFF_FUNCTIONS_EMPTY);
    write_th1d_ptr(w, &h.passed); // fPassedHistogram
    w.be_i32(h.statistic_option.code());
    write_th1d_ptr(w, &h.total); // fTotalHistogram
    w.be_f64(h.weight);
    w.end_object(te);
}

/// Serialize an `Efficiency` object to a fresh byte vector.
pub(crate) fn tefficiency_to_bytes(h: &Efficiency) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tefficiency(&mut w, h);
    w.into_vec()
}

/// Write a `TObjArray` (version 3) of `n` object-pointer elements, each emitted
/// by `elem(w, i)`.
fn write_objarray(w: &mut WBuffer, n: usize, mut elem: impl FnMut(&mut WBuffer, usize)) {
    let oa = w.begin_object(3); // TObjArray version 3
    write_object_base(w, 0);
    w.string(""); // fName
    w.be_i32(i32::saturating_from(n)); // fSize
    w.be_i32(0); // fLowerBound
    for i in 0..n {
        elem(w, i);
    }
    w.end_object(oa);
}

/// Serialize a `SparseHist` object. Layout: `THnSparseT{ SparseHist{ THnBase{
/// Named, fNdimensions, fAxes(TObjArray<Axis>), fEntries, fTsumw, fTsumw2,
/// fTsumwx, fTsumwx2 }, fChunkSize, fFilledBins, fBinContent(TObjArray<chunk>) }}`.
/// The single chunk packs every filled bin's compact coordinate and content.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the bytes of a packed coordinate"
)]
pub(crate) fn write_thnsparse(w: &mut WBuffer, h: &SparseHist) {
    let bits = h.axis_bits();
    let total_bits: u32 = bits.iter().sum();
    let single = total_bits.div_ceil(8).max(1) as usize; // fSingleCoordinateSize
    let nfilled = h.bins.len();

    let tt = w.begin_object(1); // THnSparseT<TArrayD> version 1
    let ts = w.begin_object(3); // SparseHist version 3
    let tb = w.begin_object(1); // THnBase version 1
    write_named(w, HIST_BITS, &h.name, &h.title);
    w.be_i32(i32::saturating_from(h.ndim()));
    write_objarray(w, h.ndim(), |w, i| {
        write_object_ptr(w, "TAxis", |w| write_taxis(w, &h.axes[i]));
    });
    w.be_f64(h.entries);
    w.be_f64(h.tsumw);
    w.be_f64(h.tsumw2);
    write_tarrayd(w, &h.tsumwx);
    write_tarrayd(w, &h.tsumwx2);
    w.end_object(tb);

    w.be_i32(i32::saturating_from(nfilled.max(1))); // fChunkSize (one chunk holds them all)
    w.be_i64(i64::saturating_from(nfilled)); // fFilledBins
    write_objarray(w, 1, |w, _| {
        write_object_ptr(w, "THnSparseArrayChunk", |w| {
            let chunk = w.begin_object(1); // THnSparseArrayChunk version 1
            write_object_base(w, 0);
            w.be_i32(i32::saturating_from(single)); // fSingleCoordinateSize
            w.be_i32(i32::saturating_from(nfilled * single)); // fCoordinatesSize
            w.u8(1); // char* presence flag
            for b in &h.bins {
                let coord = h.pack(&b.coords, &bits);
                for k in (0..single).rev() {
                    w.u8((coord >> (8 * k)) as u8); // big-endian
                }
            }
            let contents: Vec<f64> = h.bins.iter().map(|b| b.content).collect();
            write_object_ptr(w, "TArrayD", |w| write_tarrayd(w, &contents)); // fContent
            w.be_i32(0); // fSumw2: null pointer
            w.end_object(chunk);
        });
    });
    w.end_object(ts);
    w.end_object(tt);
}

/// Serialize a `SparseHist` object to a fresh byte vector.
pub(crate) fn thnsparse_to_bytes(h: &SparseHist) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_thnsparse(&mut w, h);
    w.into_vec()
}

/// The number of partition cells per axis ROOT uses by default (`fCellX`/`fCellY`).
const POLY_CELLS_PER_AXIS: i32 = 25;
/// `kMustCleanup` — the `fBits` value ROOT writes for a `Graph`'s `Named`.
const GRAPH_BITS: u32 = 0x0000_0400;

/// Serialize a `PolyHist` object (with its byte-count/version header).
///
/// The polygon bins are written **in full inside `fBins`**, and the `fCells`
/// spatial-lookup grid is written empty (625 empty `TList`s). That is a valid
/// ROOT serialization — the bins are first-seen in `fBins`, so no object
/// back-references are needed — and reproduces every bin faithfully; only the
/// fast-fill grid is left unpopulated (a re-fill in ROOT would need to be
/// rebuilt, which oxiroot does not do).
pub(crate) fn write_th2poly(w: &mut WBuffer, h: &PolyHist) {
    let mut bins: Vec<&PolyBin> = h.bins.iter().collect();
    bins.sort_by_key(|b| b.number);
    let n = bins.len();
    let ncells_grid = (POLY_CELLS_PER_AXIS * POLY_CELLS_PER_AXIS) as usize; // fNCells

    let tp = w.begin_object(3); // PolyHist version 3
    let th2 = w.begin_object(5); // Hist2D version 5

    // Hist1D base. fNcells is the polygon-bin count plus the 9 over/underflow
    // regions; fSumw2 is left empty so ROOT falls back to sqrt(content) errors.
    let zaxis = Axis::new("zaxis", 1, 0.0, 1.0);
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &zaxis,
            ncells: i32::saturating_from(n + 9),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &[],
            functions: &[],
        },
    );
    w.be_f64(1.0); // fScalefactor
    w.be_f64(h.tsumwy);
    w.be_f64(h.tsumwy2);
    w.be_f64(h.tsumwxy);
    w.end_object(th2);

    // PolyHist members.
    for &o in &h.overflow {
        w.be_f64(o); // fOverflow[9]
    }
    w.be_i32(POLY_CELLS_PER_AXIS); // fCellX
    w.be_i32(POLY_CELLS_PER_AXIS); // fCellY
    w.be_i32(i32::saturating_from(ncells_grid)); // fNCells

    // fCells (TStreamerLoop): a {byte count, version} header then `ncells_grid`
    // empty TLists. ROOT/uproot skip the 6-byte header; we patch the byte count.
    let cells_bc = w.reserve::<4>();
    let cells_start = w.len();
    w.be_u16(10); // streamloop version (constant in ROOT's PolyHist)
    for _ in 0..ncells_grid {
        write_empty_tlist(w);
    }
    let cells_len = u32::saturating_from(w.len() - cells_start);
    w.patch_be_u32(cells_bc, K_BYTE_COUNT_MASK | cells_len);

    let step_x = (h.xaxis.xmax - h.xaxis.xmin) / f64::from(POLY_CELLS_PER_AXIS);
    let step_y = (h.yaxis.xmax - h.yaxis.xmin) / f64::from(POLY_CELLS_PER_AXIS);
    w.be_f64(step_x); // fStepX
    w.be_f64(step_y); // fStepY

    // fIsEmpty / fCompletelyInside: bool* //[fNCells], each a presence marker
    // byte then one byte per cell. Every cell is empty and not fully inside a bin.
    w.u8(1);
    for _ in 0..ncells_grid {
        w.u8(1); // fIsEmpty[i] = true
    }
    w.u8(1);
    for _ in 0..ncells_grid {
        w.u8(0); // fCompletelyInside[i] = false
    }
    w.u8(0); // fFloat = false

    // fBins: a TList* holding every bin in full (first and only occurrence).
    write_object_ptr(w, "TList", |w| {
        let tl = w.begin_object(5); // TList version 5
        write_object_base(w, TLIST_BITS);
        w.string(""); // fName
        w.be_i32(i32::saturating_from(n)); // fSize
        for b in &bins {
            write_object_ptr(w, "TH2PolyBin", |w| write_polybin(w, b));
            w.string(""); // per-element option string
        }
        w.end_object(tl);
    });

    w.end_object(tp);
}

/// Serialize one `TH2PolyBin` body (its object-pointer header is written by the
/// caller). Layout: `TObject, fChanged, fNumber, fPoly(Graph*), fArea,
/// fContent, fXmin, fYmin, fXmax, fYmax`.
fn write_polybin(w: &mut WBuffer, b: &PolyBin) {
    let bin = w.begin_object(1); // TH2PolyBin version 1
    write_object_base(w, 0);
    w.u8(1); // fChanged
    w.be_i32(b.number);
    write_object_ptr(w, "TGraph", |w| {
        write_tgraph_base(w, "Graph", "Graph", &b.x, &b.y, None, &[])
    }); // fPoly
    w.be_f64(b.area);
    w.be_f64(b.content);
    w.be_f64(b.xmin);
    w.be_f64(b.ymin);
    w.be_f64(b.xmax);
    w.be_f64(b.ymax);
    w.end_object(bin);
}

/// Serialize a `Graph` base object (version 5): `Named`, the three `TAtt`
/// bases, `fNpoints`/`fX`/`fY`, and the trailer ROOT writes for a graph (the
/// `fFunctions` list, the `fHistogram` display frame or a null pointer,
/// `fMinimum`/`fMaximum` of `-1111`, and an empty `fOption`). Shared by the
/// standalone graph writer and `PolyHist`'s polygon `fPoly`.
fn write_tgraph_base(
    w: &mut WBuffer,
    name: &str,
    title: &str,
    x: &[f64],
    y: &[f64],
    histogram: Option<&Hist1D>,
    functions: &[GraphFunction],
) {
    let g = w.begin_object(5); // Graph version 5
    write_named(w, GRAPH_BITS, name, title);

    let line = w.begin_object(2); // TAttLine
    w.be_i16(1); // fLineColor
    w.be_i16(1); // fLineStyle
    w.be_i16(1); // fLineWidth
    w.end_object(line);
    let fill = w.begin_object(2); // TAttFill
    w.be_i16(0); // fFillColor
    w.be_i16(1000); // fFillStyle
    w.end_object(fill);
    let marker = w.begin_object(3); // TAttMarker
    w.be_i16(1); // fMarkerColor
    w.be_i16(1); // fMarkerStyle
    w.be_f32(1.0); // fMarkerSize
    w.end_object(marker);

    let npoints = x.len().min(y.len());
    w.be_i32(i32::saturating_from(npoints)); // fNpoints
    w.u8(1); // fX presence marker
    for &v in &x[..npoints] {
        w.be_f64(v);
    }
    w.u8(1); // fY presence marker
    for &v in &y[..npoints] {
        w.be_f64(v);
    }
    if functions.is_empty() {
        write_object_ptr(w, "TList", write_empty_tlist); // fFunctions (empty)
    } else {
        write_functions(w, functions); // fFunctions (TList<Func1D>)
    }
    match histogram {
        Some(h) => write_th1f_ptr(w, h), // fHistogram (real TH1F* display frame)
        None => w.be_u32(0),             // fHistogram (null TH1F*)
    }
    w.be_f64(-1111.0); // fMinimum
    w.be_f64(-1111.0); // fMaximum
    w.string(""); // fOption
    w.end_object(g);
}

/// Write a non-empty `fFunctions` `TList`, each element a `Func1D` object pointer
/// followed by its (empty) option `TString`.
fn write_functions(w: &mut WBuffer, functions: &[GraphFunction]) {
    // ROOT writes 0 bits for a graph's embedded fFunctions list.
    write_object_ptr(w, "TList", |w| write_function_list(w, 0, functions));
}

/// Write a `TList` of functions as it stands (no class tag in front): each
/// element a `TF1` object pointer followed by its (empty) option `TString`.
fn write_function_list(w: &mut WBuffer, bits: u32, functions: &[GraphFunction]) {
    let tl = w.begin_object(5); // TList version 5
    write_object_base(w, bits);
    w.string(""); // fName
    w.be_i32(i32::saturating_from(functions.len())); // fSize
    for f in functions {
        write_object_ptr(w, "TF1", |w| f.write_tf1_body(w, 1, 100));
        w.string(""); // per-element option string
    }
    w.end_object(tl);
}

/// Serialize a `PolyHist` object to a fresh byte vector.
pub(crate) fn th2poly_to_bytes(h: &PolyHist) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_th2poly(&mut w, h);
    w.into_vec()
}

/// Write a `Double_t* //[n]` member: a presence marker byte then exactly `n`
/// doubles, taking them from `data` and zero-padding if it is shorter.
fn write_basic_array(w: &mut WBuffer, data: &[f64], n: usize) {
    w.u8(1); // present
    for i in 0..n {
        w.be_f64(data.get(i).copied().unwrap_or(0.0));
    }
}

/// Serialize a [`Graph`] (or its `TGraphErrors`/`TGraphAsymmErrors` variant,
/// chosen by [`Graph::errors`]) with its byte-count/version header.
pub(crate) fn write_tgraph(w: &mut WBuffer, g: &Graph) {
    let n = g.len();
    let x = &g.x[..n];
    let y = &g.y[..n];
    match &g.errors {
        GraphErrors::Plain => write_tgraph_base(
            w,
            &g.name,
            &g.title,
            x,
            y,
            g.histogram.as_ref(),
            &g.functions,
        ),
        GraphErrors::Symmetric { ex, ey } => {
            let t = w.begin_object(3); // TGraphErrors version 3
            write_tgraph_base(
                w,
                &g.name,
                &g.title,
                x,
                y,
                g.histogram.as_ref(),
                &g.functions,
            );
            write_basic_array(w, ex, n);
            write_basic_array(w, ey, n);
            w.end_object(t);
        }
        GraphErrors::Asymmetric {
            ex_low,
            ex_high,
            ey_low,
            ey_high,
        } => {
            let t = w.begin_object(3); // TGraphAsymmErrors version 3
            write_tgraph_base(
                w,
                &g.name,
                &g.title,
                x,
                y,
                g.histogram.as_ref(),
                &g.functions,
            );
            write_basic_array(w, ex_low, n);
            write_basic_array(w, ex_high, n);
            write_basic_array(w, ey_low, n);
            write_basic_array(w, ey_high, n);
            w.end_object(t);
        }
    }
}

/// Serialize a [`Graph`] object to a fresh byte vector.
pub(crate) fn tgraph_to_bytes(g: &Graph) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tgraph(&mut w, g);
    w.into_vec()
}

/// Serialize a [`Graph2D`] (version 1): the three `TAtt` bases, the scalar
/// display parameters at ROOT's defaults, the `fX`/`fY`/`fZ` arrays, and an empty
/// `fFunctions` (the `fHistogram` frame is transient and not persisted).
fn write_tgraph2d(w: &mut WBuffer, g: &Graph2D) {
    let n = g.len();
    let obj = w.begin_object(1); // Graph2D version 1
    write_named(w, GRAPH_BITS, &g.name, &g.title);

    let line = w.begin_object(2); // TAttLine
    w.be_i16(1);
    w.be_i16(1);
    w.be_i16(1);
    w.end_object(line);
    let fill = w.begin_object(2); // TAttFill
    w.be_i16(0);
    w.be_i16(1000);
    w.end_object(fill);
    let marker = w.begin_object(3); // TAttMarker
    w.be_i16(1);
    w.be_i16(1);
    w.be_f32(1.0);
    w.end_object(marker);

    w.be_i32(i32::saturating_from(n)); // fNpoints
    w.be_i32(40); // fNpx (ROOT default)
    w.be_i32(40); // fNpy
    w.be_i32(100_000); // fMaxIter
    write_basic_array(w, &g.x, n); // fX
    write_basic_array(w, &g.y, n); // fY
    write_basic_array(w, &g.z, n); // fZ
    w.be_f64(-1111.0); // fMinimum
    w.be_f64(-1111.0); // fMaximum
    w.be_f64(0.0); // fMargin
    w.be_f64(0.0); // fZout
    write_object_ptr(w, "TList", write_empty_tlist); // fFunctions
    w.u8(0); // fUserHisto (false)
    w.end_object(obj);
}

/// Serialize a [`Graph2D`] object to a fresh byte vector.
pub(crate) fn tgraph2d_to_bytes(g: &Graph2D) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tgraph2d(&mut w, g);
    w.into_vec()
}

impl WriteRoot for Graph2D {
    fn root_class(&self) -> String {
        "TGraph2D".to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        &self.title
    }
    fn to_root_bytes(&self) -> Vec<u8> {
        tgraph2d_to_bytes(self)
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        hist_streamer_classes(&["TGraph2D"])
    }
}

/// Write an *objectwise* `vector<TArrayD>` (version `0x000a`): a count, then each
/// layer as a `TArrayD` (`fN` + `fN` doubles), zero-padded to `n`.
fn write_vector_tarrayd(w: &mut WBuffer, layers: &[Vec<f64>], n: usize) {
    let t = w.begin_object(0x000a);
    w.be_i32(i32::saturating_from(layers.len()));
    for layer in layers {
        w.be_i32(i32::saturating_from(n));
        for i in 0..n {
            w.be_f64(layer.get(i).copied().unwrap_or(0.0));
        }
    }
    w.end_object(t);
}

/// Write a *memberwise* `vector<TAtt*>` (version `0x400a` — the `0x4000`
/// `kStreamedMemberWise` bit set): the element class version, a count, then each
/// `i16` member emitted across all `count` elements in turn. `members` lists the
/// per-element value for each member (all layers share ROOT's default attrs).
fn write_memberwise_att(w: &mut WBuffer, elem_version: i16, count: usize, members: &[i16]) {
    let t = w.begin_object(0x400a);
    w.be_i16(elem_version);
    w.be_i32(i32::saturating_from(count));
    for &m in members {
        for _ in 0..count {
            w.be_i16(m);
        }
    }
    w.end_object(t);
}

/// Serialize a [`MultiErrorGraph`] (version 1): the full `Graph` base, the
/// `fNYErrors`/`fSumErrorsMode` scalars, the `fExL`/`fExH` arrays, the `fEyL`/
/// `fEyH` objectwise `vector<TArrayD>`, and the `fAttFill`/`fAttLine` memberwise
/// attribute vectors at ROOT's per-layer defaults.
fn write_tgraphmultierrors(w: &mut WBuffer, g: &MultiErrorGraph) {
    let n = g.len();
    let layers = g.ey_low.len();
    let obj = w.begin_object(1); // MultiErrorGraph version 1
    write_tgraph_base(w, &g.name, &g.title, &g.x[..n], &g.y[..n], None, &[]);
    w.be_i32(i32::saturating_from(layers)); // fNYErrors
    w.be_i32(g.sum_errors_mode.code()); // fSumErrorsMode
    write_basic_array(w, &g.ex_low, n); // fExL
    write_basic_array(w, &g.ex_high, n); // fExH
    write_vector_tarrayd(w, &g.ey_low, n); // fEyL
    write_vector_tarrayd(w, &g.ey_high, n); // fEyH
    // fAttFill: fFillColor, fFillStyle (ROOT's per-layer defaults 19 / 1001).
    write_memberwise_att(w, 2, layers, &[19, 1001]);
    // fAttLine: fLineColor, fLineStyle, fLineWidth (defaults 1 / 1 / 1).
    write_memberwise_att(w, 2, layers, &[1, 1, 1]);
    w.end_object(obj);
}

/// Serialize a [`MultiErrorGraph`] object to a fresh byte vector.
pub(crate) fn tgraphmultierrors_to_bytes(g: &MultiErrorGraph) -> Vec<u8> {
    let mut w = WBuffer::new();
    write_tgraphmultierrors(&mut w, g);
    w.into_vec()
}

impl WriteRoot for MultiErrorGraph {
    fn root_class(&self) -> String {
        "TGraphMultiErrors".to_string()
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        &self.title
    }
    fn to_root_bytes(&self) -> Vec<u8> {
        tgraphmultierrors_to_bytes(self)
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        hist_streamer_classes(&["TGraphMultiErrors"])
    }
}

fn write_th1_base(w: &mut WBuffer, h: &Hist1D) {
    write_th1_core(
        w,
        &Th1Core {
            name: &h.name,
            title: &h.title,
            xaxis: &h.xaxis,
            yaxis: &h.yaxis,
            zaxis: &h.zaxis,
            ncells: h.ncells(),
            entries: h.entries,
            tsumw: h.tsumw,
            tsumw2: h.tsumw2,
            tsumwx: h.tsumwx,
            tsumwx2: h.tsumwx2,
            sumw2: &h.sumw2,
            functions: &h.functions,
        },
    );
}

/// What the `TH1` base of a histogram class holds, whichever class it is.
#[derive(Clone, Copy)]
struct Th1Core<'a> {
    name: &'a str,
    title: &'a str,
    xaxis: &'a Axis,
    yaxis: &'a Axis,
    zaxis: &'a Axis,
    /// `fNcells`: every cell, flow included.
    ncells: i32,
    entries: f64,
    tsumw: f64,
    tsumw2: f64,
    tsumwx: f64,
    tsumwx2: f64,
    /// `fSumw2`: per-cell Σw², or what a profile keeps there.
    sumw2: &'a [f64],
    /// `fFunctions`: the functions stored with the histogram.
    functions: &'a [GraphFunction],
}

/// Write the shared `Hist1D` base object (version 8) used by every histogram
/// class. The dimension-specific stat sums (y/z) and the data `TArray` are
/// written by the caller after this returns.
fn write_th1_core(w: &mut WBuffer, core: &Th1Core<'_>) {
    let Th1Core {
        name,
        title,
        xaxis,
        yaxis,
        zaxis,
        ncells,
        entries,
        tsumw,
        tsumw2,
        tsumwx,
        tsumwx2,
        sumw2: fsumw2,
        functions,
    } = *core;
    let th1 = w.begin_object(8); // Hist1D version 8

    write_named(w, HIST_BITS, name, title);
    write_attline(w);
    write_attfill(w);
    write_attmarker(w);

    w.be_i32(ncells);
    write_taxis(w, xaxis);
    write_taxis(w, yaxis);
    write_taxis(w, zaxis);
    w.be_i16(0); // fBarOffset
    w.be_i16(1000); // fBarWidth
    w.be_f64(entries);
    w.be_f64(tsumw);
    w.be_f64(tsumw2);
    w.be_f64(tsumwx);
    w.be_f64(tsumwx2);
    w.be_f64(-1111.0); // fMaximum
    w.be_f64(-1111.0); // fMinimum
    w.be_f64(0.0); // fNormFactor
    write_tarrayd(w, &[]); // fContour
    write_tarrayd(w, fsumw2); // fSumw2 (per-bin sum of squared weights)
    w.string(""); // fOption
    if functions.is_empty() {
        write_empty_tlist(w); // fFunctions
    } else {
        write_function_list(w, TLIST_BITS, functions);
    }
    w.be_i32(0); // fBufferSize
    w.u8(0); // fBuffer (null pointer-to-array marker)
    w.be_i32(0); // fBinStatErrOpt
    w.be_i32(2); // fStatOverflows

    w.end_object(th1);
}

fn write_attline(w: &mut WBuffer) {
    let t = w.begin_object(2);
    w.be_i16(602); // fLineColor
    w.be_i16(1); // fLineStyle
    w.be_i16(1); // fLineWidth
    w.end_object(t);
}

fn write_attfill(w: &mut WBuffer) {
    let t = w.begin_object(2);
    w.be_i16(0); // fFillColor
    w.be_i16(1001); // fFillStyle
    w.end_object(t);
}

fn write_attmarker(w: &mut WBuffer) {
    let t = w.begin_object(2);
    w.be_i16(1); // fMarkerColor
    w.be_i16(1); // fMarkerStyle
    w.be_f32(1.0); // fMarkerSize
    w.end_object(t);
}

fn write_taxis(w: &mut WBuffer, ax: &Axis) {
    let t = w.begin_object(10); // Axis version 10
    write_named(w, AXIS_BITS, &ax.name, &ax.title);

    // TAttAxis base (drawing defaults).
    let att = w.begin_object(4);
    w.be_i32(510); // fNdivisions
    w.be_i16(1); // fAxisColor
    w.be_i16(1); // fLabelColor
    w.be_i16(42); // fLabelFont
    w.be_f32(0.005); // fLabelOffset
    w.be_f32(0.035); // fLabelSize
    w.be_f32(0.03); // fTickLength
    w.be_f32(1.0); // fTitleOffset
    w.be_f32(0.035); // fTitleSize
    w.be_i16(1); // fTitleColor
    w.be_i16(42); // fTitleFont
    w.end_object(att);

    w.be_i32(ax.nbins);
    w.be_f64(ax.xmin);
    w.be_f64(ax.xmax);
    write_tarrayd(w, &ax.xbins); // fXbins
    w.be_i32(0); // fFirst
    w.be_i32(0); // fLast
    w.be_u16(0); // fBits2
    w.u8(u8::from(ax.time_display)); // fTimeDisplay
    w.string(&ax.time_format); // fTimeFormat
    if ax.labels.iter().any(|l| !l.is_empty()) {
        write_labels(w, &ax.labels); // fLabels (THashList*)
    } else {
        w.be_u32(0); // fLabels (null THashList*)
    }
    w.be_u32(0); // fModLabs (null TList*)
    w.end_object(t);
}

/// Write `fLabels`: a `THashList*` of `ObjString`, one per labelled bin, with
/// the 1-based bin number in each `ObjString`'s `fUniqueID` — the layout ROOT
/// writes for an alphanumeric axis. (Each entry names its class with
/// `kNewClassTag`; ROOT and uproot read that as well as ROOT's own back-refs.)
fn write_labels(w: &mut WBuffer, labels: &[String]) {
    write_object_ptr(w, "THashList", |w| {
        let hl = w.begin_object(5); // THashList (a TList, version 5)
        write_object_base(w, 0);
        w.string(""); // fName
        let present: Vec<(usize, &String)> = labels
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.is_empty())
            .map(|(i, l)| (i + 1, l)) // 1-based bin number
            .collect();
        w.be_i32(i32::saturating_from(present.len())); // fSize
        for (bin, label) in present {
            write_object_ptr(w, "TObjString", |w| {
                let ts = w.begin_object(1); // ObjString version 1
                w.be_u16(1); // TObject version
                w.be_u32(u32::saturating_from(bin)); // fUniqueID = bin number
                w.be_u32(0); // fBits
                w.string(label); // fString
                w.end_object(ts);
            });
            w.string(""); // per-element option string
        }
        w.end_object(hl);
    });
}

fn write_empty_tlist(w: &mut WBuffer) {
    let t = w.begin_object(5); // TList version 5
    write_object_base(w, TLIST_BITS);
    w.string(""); // fName
    w.be_i32(0); // fSize
    w.end_object(t);
}

/// Write a `TArrayD` base inline (a count followed by that many doubles).
fn write_tarrayd(w: &mut WBuffer, data: &[f64]) {
    write_tarray(w, data, BinContentType::F64);
}

/// Write a `TArray{D,F,I,S,C,L64}` base inline: a count, then that many
/// values of the type `contents` names. The bin contents are `f64` in memory;
/// a `TH*F`, `TH*I`, … narrows them to its own type, as ROOT does.
#[allow(
    clippy::cast_possible_truncation,
    reason = "contents stored in the class type (TH1F, TH1S, TH1I, TH1C, TH1L): the narrowing is the format, as in ROOT"
)]
fn write_tarray(w: &mut WBuffer, data: &[f64], contents: BinContentType) {
    w.be_i32(i32::saturating_from(data.len()));
    for &d in data {
        match contents {
            BinContentType::F64 => w.be_f64(d),
            BinContentType::F32 => w.be_f32(d as f32),
            BinContentType::I32 => w.be_i32(d as i32),
            BinContentType::I16 => w.be_i16(d as i16),
            BinContentType::I8 => w.u8((d as i8).cast_unsigned()),
            BinContentType::I64 => w.be_i64(d as i64),
        }
    }
}
