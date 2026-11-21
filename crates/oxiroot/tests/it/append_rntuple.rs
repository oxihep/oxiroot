//! Append mode for a file that contains an `RNTuple`. The `RNTuple`'s anchor and
//! page locators hold absolute file offsets, so append-in-place must not move
//! it; only new keys are written after the existing content. Both the `RNTuple`
//! and the appended histogram read back (separately verified against ROOT C++
//! and uproot).

use crate::support::NtupleObject;
use oxiroot::FileReader;
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::{Field, FieldValues};
use oxiroot::prelude::*;

#[test]
fn appends_a_histogram_to_an_rntuple_file() {
    let out = std::env::temp_dir().join("oxiroot_append_rntuple.root");

    let fields = vec![
        Field::f64("mass", vec![91.2, 125.0, 80.4]),
        Field::i32("n", vec![1, 2, 3]),
    ];
    FileWriter::create(&out)
        .add(NtupleObject::new("events", fields))
        .compression(Compression::Stored)
        .write()
        .expect("write rntuple");

    let mut h = Hist::reg(4, 0.0, 4.0)
        .double()
        .named("h")
        .titled("appended");
    h.fill(0.5);
    h.fill(2.5);
    let h = h.to_root().unwrap();
    oxiroot::FileUpdater::open(&out)
        .expect("open for append")
        .add(&h)
        .compression(Compression::Stored)
        .write()
        .expect("append");

    let f = FileReader::open(&out).expect("reopen");
    let names: Vec<&str> = f.keys().iter().map(|k| k.name.as_str()).collect();
    assert!(
        names.contains(&"events") && names.contains(&"h"),
        "{names:?}"
    );

    // The RNTuple still reads — its anchor/page offsets were not relocated.
    let nt = NtupleReader::open(&f, "events").expect("rntuple survived the append");
    assert_eq!(nt.entries(), 3);
    assert_eq!(
        nt.field("mass").values().unwrap(),
        FieldValues::F64(vec![91.2, 125.0, 80.4])
    );
    assert_eq!(
        nt.field("n").values().unwrap(),
        FieldValues::I32(vec![1, 2, 3])
    );
    assert_eq!(f.get::<Hist1D>("h").unwrap(), h, "appended histogram");

    let _ = std::fs::remove_file(&out);
}
