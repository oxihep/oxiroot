//! The memory-mapped read path (`mmap` feature) must yield the same parsed
//! container as the in-memory path.
#![cfg(feature = "mmap")]

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::PathBuf;

use oxiroot_io_core::FileReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn open_mmap_matches_open() {
    let path = fixture("th1d_uncompressed.root");
    let owned = FileReader::open(&path).expect("open");
    // SAFETY: the fixture is not modified while it is mapped.
    #[allow(unsafe_code)]
    let mapped = unsafe { FileReader::open_mmap(&path) }.expect("open_mmap");

    assert_eq!(mapped.size(), owned.size(), "same length");
    let whole = |f: &FileReader| {
        f.read_at(0, usize::saturating_from(f.size()))
            .expect("read whole")
    };
    assert_eq!(whole(&mapped), whole(&owned), "same bytes");
    let names =
        |f: &FileReader| -> Vec<String> { f.keys().iter().map(|k| k.name.clone()).collect() };
    assert_eq!(names(&mapped), names(&owned), "same keys");
}
