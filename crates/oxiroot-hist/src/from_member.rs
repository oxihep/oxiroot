//! [`FromMember`] for the histogram-family types, so
//! [`ObjList::items`](oxiroot_io_core::ObjList::items) and
//! [`ObjMap::get`](oxiroot_io_core::ObjMap::get) can pull histograms and graphs out
//! of a collection.

use oxiroot_io_core::{FromMember, Result};

use crate::graph::{Graph, decode_tgraph};
use crate::hist1d::{Hist1D, decode_th1};
use crate::hist2d::{Hist2D, decode_th2};
use crate::hist3d::{Hist3D, decode_th3};

impl FromMember for Hist1D {
    fn from_member(class: &str, bytes: &[u8]) -> Option<Result<Self>> {
        class
            .starts_with("TH1")
            .then(|| decode_th1((class.to_string(), bytes.to_vec())))
    }
}
impl FromMember for Hist2D {
    fn from_member(class: &str, bytes: &[u8]) -> Option<Result<Self>> {
        (class.starts_with("TH2") && class != "TH2Poly")
            .then(|| decode_th2((class.to_string(), bytes.to_vec())))
    }
}
impl FromMember for Hist3D {
    fn from_member(class: &str, bytes: &[u8]) -> Option<Result<Self>> {
        class
            .starts_with("TH3")
            .then(|| decode_th3((class.to_string(), bytes.to_vec())))
    }
}
impl FromMember for Graph {
    fn from_member(class: &str, bytes: &[u8]) -> Option<Result<Self>> {
        matches!(class, "TGraph" | "TGraphErrors" | "TGraphAsymmErrors")
            .then(|| decode_tgraph(class, class, bytes))
    }
}
