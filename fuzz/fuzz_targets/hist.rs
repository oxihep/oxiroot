#![no_main]
//! Fuzz the typed object readers (histograms, profiles, graphs, functions,
//! collections): every reader must reject malformed bytes, never panic.
use libfuzzer_sys::fuzz_target;
use oxiroot_hist::{
    Efficiency, Graph, Graph2D, GraphStack, Hist1D, Hist2D, Hist3D, HistStack, MultiErrorGraph,
    PolyHist, Profile1D, Profile2D, Profile3D, SparseHist,
};
use oxiroot_hist_func::{Func1D, Func2D, Func3D};
use oxiroot_io_core::FileReader;
use oxiroot_io_core::{ObjList, ObjMap, ObjString, Parameter};

/// Try every typed reader on `name`; each checks the key's class first, so the
/// matching one does the real parsing.
fn read_all(f: &FileReader, name: &str) {
    macro_rules! try_read {
        ($($t:ty),+ $(,)?) => {$( let _ = f.get::<$t>(name); )+};
    }
    try_read!(
        Hist1D,
        Hist2D,
        Hist3D,
        Profile1D,
        Profile2D,
        Profile3D,
        Efficiency,
        SparseHist,
        PolyHist,
        Graph,
        Graph2D,
        MultiErrorGraph,
        Func1D,
        Func2D,
        Func3D,
        ObjList,
        ObjMap,
        HistStack,
        GraphStack,
        ObjString,
        Parameter,
    );
}

fuzz_target!(|data: &[u8]| {
    let Ok(f) = FileReader::from_bytes(data.to_vec()) else {
        return;
    };
    let keys: Vec<(String, String)> = f
        .keys()
        .iter()
        .take(8)
        .map(|k| (k.name.clone(), k.class_name.clone()))
        .collect();
    for (name, class) in &keys {
        read_all(&f, name);
        if class == "TDirectory" || class == "TDirectoryFile" {
            let _ = f.get::<Hist1D>(&format!("{}/h", name));
        }
    }
});
