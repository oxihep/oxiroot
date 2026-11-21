//! Write nested collection fields — `std::vector<std::string>`,
//! `std::vector<std::vector<T>>`, a vector of records, and a top-level record —
//! then read them back through our own reader.
use oxiroot_io_core::Column;

use oxiroot_io_core::{Compression, FileReader};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn round_trip(fields: &[Field], tag: &str) -> FileReader {
    let out = std::env::temp_dir().join(format!("oxiroot_write_nested_{tag}.root"));
    crate::support::write_rntuple_file(&out, "ntpl", fields, Compression::Stored).expect("write");
    FileReader::open(&out).expect("reopen")
}

#[test]
fn writes_vector_of_strings() {
    let vs = vec![
        vec![],
        vec!["a".to_string()],
        vec!["b".to_string(), "c".to_string()],
    ];
    let fields = vec![Field::new("vs", vs.clone())];
    let file = round_trip(&fields, "vs");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    assert_eq!(ntpl.field("vs").values().expect("vs"), Column::from(vs));
}

#[test]
fn writes_vector_of_vectors() {
    let vvi = vec![vec![], vec![vec![1]], vec![vec![2], vec![3, 3]]];
    let fields = vec![Field::new("vvi", vvi)];
    let file = round_trip(&fields, "vvi");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    assert_eq!(
        ntpl.field("vvi").values().expect("vvi"),
        Column::List {
            offsets: vec![0, 0, 1, 3],
            values: Box::new(Column::from(vec![vec![1], vec![2], vec![3, 3]])),
        }
    );
}

#[test]
fn writes_vector_of_records() {
    // 3 entries of std::pair<int32,double>: [], [(10,1.5)], [(20,2.5),(21,3.5)].
    let vp = Column::List {
        offsets: vec![0, 0, 1, 3],
        values: Box::new(Column::Struct(vec![
            ("_0".to_string(), Column::I32(vec![10, 20, 21])),
            ("_1".to_string(), Column::F64(vec![1.5, 2.5, 3.5])),
        ])),
    };
    let fields = vec![Field::new("vp", vp)];
    let file = round_trip(&fields, "vp");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    assert_eq!(
        ntpl.field("vp").values().expect("vp"),
        Column::List {
            offsets: vec![0, 0, 1, 3],
            values: Box::new(Column::Struct(vec![
                ("_0".to_string(), Column::I32(vec![10, 20, 21])),
                ("_1".to_string(), Column::F64(vec![1.5, 2.5, 3.5])),
            ])),
        }
    );
}

#[test]
fn writes_a_top_level_record() {
    // A struct field { _0: int32, _1: double } directly at top level.
    let rec = Column::Struct(vec![
        ("_0".to_string(), Column::I32(vec![1, 2, 3])),
        ("_1".to_string(), Column::F64(vec![1.5, 2.5, 3.5])),
    ]);
    let fields = vec![Field::new("p", rec)];
    let file = round_trip(&fields, "rec");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    assert_eq!(ntpl.entries(), 3);
    assert_eq!(
        ntpl.field("p").values().expect("p"),
        Column::Struct(vec![
            ("_0".to_string(), Column::I32(vec![1, 2, 3])),
            ("_1".to_string(), Column::F64(vec![1.5, 2.5, 3.5])),
        ])
    );
}
