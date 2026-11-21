//! Positioned local reads (a `FileSource`, which `FileReader::open` uses for a
//! large file) must parse and read byte-for-byte as a file held in memory does.

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::PathBuf;

use oxiroot_io_core::{FileReader, FileSource};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn positioned_reads_match_a_file_in_memory() {
    for name in ["th1d_uncompressed.root", "th1d_zstd.root"] {
        let path = fixture(name);
        let resident = FileReader::from_bytes(std::fs::read(&path).unwrap()).expect("open");
        let ranged = FileReader::from_source(Box::new(FileSource::open(&path).unwrap()))
            .expect("positioned");

        assert_eq!(ranged.size(), resident.size(), "{name}: same length");

        let names =
            |f: &FileReader| -> Vec<String> { f.keys().iter().map(|k| k.name.clone()).collect() };
        assert_eq!(names(&ranged), names(&resident), "{name}: same keys");

        // Every key's payload bytes must match, fetched positionally vs sliced.
        for k in resident.keys() {
            assert_eq!(
                ranged.key_payload(k).unwrap(),
                resident.key_payload(k).unwrap(),
                "{name}: key {:?} payload",
                k.name
            );
        }

        // The whole file, read range-by-range, equals the resident buffer.
        let whole = |f: &FileReader| f.read_at(0, usize::saturating_from(f.size())).unwrap();
        assert_eq!(whole(&ranged), whole(&resident), "{name}: whole-file bytes");
    }
}
