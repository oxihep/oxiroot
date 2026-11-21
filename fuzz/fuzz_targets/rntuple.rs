#![no_main]
//! Fuzz the RNTuple read path (anchor → envelopes → page decode → fields).
use libfuzzer_sys::fuzz_target;
use oxiroot_io_core::FileReader;
use oxiroot_rntuple::NtupleReader;

fuzz_target!(|data: &[u8]| {
    if let Ok(f) = FileReader::from_bytes(data.to_vec())
        && let Ok(ntpl) = NtupleReader::open(&f, "ntpl")
    {
        let names: Vec<String> = ntpl.field_names().iter().map(|s| s.to_string()).collect();
        for n in &names {
            let _ = ntpl.field(n).values();
        }
    }
});
