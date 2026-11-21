//! ROOT block (de)compression throughput for every codec, on a payload shaped
//! like a `TTree` basket of doubles.
//!
//!   cargo bench -p oxiroot --bench decompress

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxiroot::compress::{compress, decompress};
use oxiroot::prelude::*;

/// 4 MiB of big-endian doubles from a smooth series with noise in the low
/// bits: compressible the way physics columns are, not trivially.
fn payload() -> Vec<u8> {
    (0..(4 << 20) / 8)
        .flat_map(|i: u64| {
            let x = (i as f64 * 1e-3).sin() * 100.0
                + (i.wrapping_mul(2_654_435_761) % 1000) as f64 * 1e-6;
            x.to_be_bytes()
        })
        .collect()
}

const CODECS: [(&str, Compression); 4] = [
    ("zlib-6", Compression::Zlib(6)),
    ("lz4-4", Compression::Lz4(4)),
    ("zstd-5", Compression::Zstd(5)),
    ("lzma-6", Compression::Lzma(6)),
];

fn bench(c: &mut Criterion) {
    let raw = payload();
    let mut g = c.benchmark_group("decompress");
    g.throughput(Throughput::Bytes(raw.len() as u64));
    g.sample_size(20);
    for (name, codec) in CODECS {
        let packed = compress(&raw, codec).expect("compress");
        g.bench_with_input(BenchmarkId::from_parameter(name), &packed, |b, packed| {
            b.iter(|| black_box(decompress(packed, raw.len()).expect("decompress")));
        });
    }
    g.finish();

    let mut g = c.benchmark_group("compress");
    g.throughput(Throughput::Bytes(raw.len() as u64));
    g.sample_size(10);
    for (name, codec) in CODECS {
        g.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| black_box(compress(&raw, codec).expect("compress")));
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
