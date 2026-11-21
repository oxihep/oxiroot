//! Hardening: corrupt/truncated `RNTuple` bytes must yield `Err`, never panic.
//! A panic anywhere below fails the test.

use std::path::PathBuf;

use oxiroot_io_core::FileReader;
use oxiroot_rntuple::NtupleReader;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("read fixture")
}

fn poke_rntuple(f: &FileReader) {
    if let Ok(ntpl) = NtupleReader::open(f, "ntpl") {
        let names: Vec<String> = ntpl
            .field_names()
            .collect::<Vec<_>>()
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        for name in names {
            let _ = ntpl.field(&name).values();
            // The cluster-bounded prefix read shares the decode path; exercise it
            // (including the cluster-slicing / view) on the malformed bytes too.
            let _ = ntpl.field(&name).range(0..2).values();
            // And the read → write round trip of the field.
            let _ = ntpl.field(&name).to_field();
        }
    }
}

#[test]
fn rntuple_byte_flips_never_panic() {
    for fix in [
        "rntuple_scalars_uncompressed.root",
        "rntuple_scalars_zstd.root",
        "rntuple_multicluster_vec.root",
    ] {
        let data = fixture(fix);
        for i in 0..data.len() {
            for v in [0x00u8, 0xff] {
                let mut c = data.clone();
                c[i] = v;
                if let Ok(f) = FileReader::from_bytes(c) {
                    poke_rntuple(&f);
                }
            }
        }
    }
}

#[test]
fn rntuple_truncation_never_panics() {
    let data = fixture("rntuple_scalars_uncompressed.root");
    for len in 0..=data.len() {
        if let Ok(f) = FileReader::from_bytes(data[..len].to_vec()) {
            poke_rntuple(&f);
        }
    }
}
