//! Write the small/medium integer encodings (Int8/UInt8/Int16/UInt16, scalar +
//! vector) and the reduced-precision reals (half / truncated / quantized), then
//! read them back through our own reader.
use oxiroot_io_core::{Column, DataType};

use oxiroot_io_core::{Compression, FileReader};
use oxiroot_rntuple::NtupleReader;
use oxiroot_rntuple::raw::Field;

fn round_trip(fields: &[Field], tag: &str) -> FileReader {
    let out = std::env::temp_dir().join(format!("oxiroot_write_coltypes_{tag}.root"));
    crate::support::write_rntuple_file(&out, "ntpl", fields, Compression::Stored).expect("write");
    FileReader::open(&out).expect("reopen")
}

#[test]
fn writes_small_integers() {
    let fields = vec![
        Field::new("i8", vec![-2_i8, -1, 0, 1, 2]),
        Field::new("u8", vec![250_u8, 251, 252, 253, 254]),
        Field::new("i16", vec![-2000_i16, -1000, 0, 1000, 2000]),
        Field::new("u16", vec![5_u16, 10005, 20005, 30005, 40005]),
        Field::new(
            "vi16",
            vec![
                vec![],
                vec![101_i16],
                vec![102, 102],
                vec![103; 3],
                vec![104; 4],
            ],
        ),
    ];
    let file = round_trip(&fields, "ints");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    let field = |n| ntpl.field(n).values().expect("read");

    assert_eq!(field("i8"), Column::I8(vec![-2, -1, 0, 1, 2]));
    assert_eq!(field("u8"), Column::U8(vec![250, 251, 252, 253, 254]));
    assert_eq!(field("i16"), Column::I16(vec![-2000, -1000, 0, 1000, 2000]));
    assert_eq!(
        field("u16"),
        Column::U16(vec![5, 10005, 20005, 30005, 40005])
    );
    assert_eq!(
        field("vi16"),
        Column::from(vec![
            vec![],
            vec![101_i16],
            vec![102, 102],
            vec![103; 3],
            vec![104; 4]
        ])
    );
}

#[test]
fn writes_reduced_precision_reals() {
    let exact = vec![0.5_f32, 1.0, 2.0, 4.0, 8.0]; // exact under half + 16-bit trunc
    let fields = vec![
        Field::new("h", exact.clone()).stored_as(DataType::Half),
        Field::new("t", exact.clone()).stored_as(DataType::Truncated { bits: 16 }),
        Field::new("q", vec![0.0_f32, 25.0, 50.0, 75.0, 100.0]).stored_as(DataType::Quantized {
            bits: 16,
            min: 0.0,
            max: 100.0,
        }),
        Field::new("q12", vec![0.0_f32, 25.0, 50.0, 75.0, 100.0]).stored_as(DataType::Quantized {
            bits: 12,
            min: 0.0,
            max: 100.0,
        }),
    ];
    let file = round_trip(&fields, "reals");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    let f32s = |n| match ntpl.field(n).values().expect("read") {
        Column::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };

    // Half and 16-bit truncation are lossless for these values.
    assert_eq!(f32s("h"), exact);
    assert_eq!(f32s("t"), exact);

    // Quantized values round-trip to within the quantization step.
    for (n, tol) in [("q", 1e-3_f32), ("q12", 3e-2)] {
        let got = f32s(n);
        for (i, (&g, &w)) in got.iter().zip(&[0.0, 25.0, 50.0, 75.0, 100.0]).enumerate() {
            assert!((g - w).abs() <= tol, "{n}[{i}] = {g}, want ~{w}");
        }
    }
}

#[test]
fn writes_a_variant_field() {
    // std::variant<int32_t, float>, alternating int (entries 0,2,4) / float (1,3).
    // The dense alternatives hold only their active values, in entry order.
    let fields = vec![Field::new(
        "v",
        Column::variant(
            vec![Column::I32(vec![0, 20, 40]), Column::F32(vec![1.5, 3.5])],
            vec![1, 2, 1, 2, 1],
        ),
    )];
    let file = round_trip(&fields, "variant");
    let ntpl = NtupleReader::open(&file, "ntpl").expect("open");
    assert_eq!(ntpl.entries(), 5);

    // Reads back as the same columnar variant the read path produces.
    assert_eq!(
        ntpl.field("v").values().expect("read v"),
        Column::Union {
            alternatives: vec![
                ("_0".to_string(), Column::I32(vec![0, 20, 40])),
                ("_1".to_string(), Column::F32(vec![1.5, 3.5])),
            ],
            tags: vec![1, 2, 1, 2, 1],
            indices: vec![0, 0, 1, 1, 2],
        }
    );
}
