//! Merging ROOT files — a pure-Rust `hadd`. Run with:
//!
//! ```sh
//! cargo run -p oxiroot --example merge
//! ```
//!
//! A `Merger` combines several ROOT files the way
//! ROOT's `hadd` tool does: histograms are summed bin-by-bin, and `TTree` /
//! `RNTuple` entries are concatenated. The ROOT command this mirrors:
//!
//! ```sh
//! hadd merged.root run1.root run2.root
//! ```

use oxiroot::file::ObjString;
use oxiroot::hadd::Merger;
use oxiroot::prelude::*;

fn main() -> oxiroot::Result<()> {
    let dir = std::env::temp_dir();
    let path = |name: &str| dir.join(name);

    // --- Two histogram files (as if from two batch jobs) ---------------------
    for (file, fills) in [
        ("run1.root", [5.0, 15.0, 15.0]),
        ("run2.root", [25.0, 35.0, 45.0]),
    ] {
        let mut h = Hist::reg(10, 0.0, 100.0)
            .double()
            .named("pt")
            .titled("p_{T}");
        for x in fills {
            h.fill(x);
        }
        FileWriter::create(path(file))
            .add(&h.to_root()?)
            .add(&ObjString::new("skim v2").named("provenance"))
            .write()?;
    }

    // Sum the histograms; the ObjString is copied from the first file.
    let report = Merger::new()
        .inputs([path("run1.root"), path("run2.root")])
        .merge(path("hists.root"))?;
    println!("{report}");

    // --- Two tree files, entries concatenated --------------------------------
    for (file, mass) in [
        ("evt1.root", vec![91.2, 125.0]),
        ("evt2.root", vec![80.4, 172.0, 4.18]),
    ] {
        FileWriter::create(path(file))
            .add(NamedBatch::new(
                "Events",
                Format::Tree,
                Batch::new().column("mass", mass)?,
            ))
            .write()?;
    }

    // The Merger builder is the composable form of `merge_files`.
    let report = Merger::new()
        .input(path("evt1.root"))
        .input(path("evt2.root"))
        .merge(path("events.root"))?;
    println!("{report}");

    Ok(())
}
