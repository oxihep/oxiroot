//! Write throughput: the same 1M-entry dataset — two scalars and a
//! `std::vector<double>` — written to memory as a `TTree` and an RNTuple, by
//! `DatasetWriter` from a `Batch` and by the format's own writer from
//! per-entry rows.
//!
//!   cargo bench -p oxiroot --bench write

use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use oxiroot::ntuple::{Field, NtupleWriter};
use oxiroot::prelude::*;
use oxiroot::tree::{Branch, TreeWriter};

const ENTRIES: usize = 1_000_000;

fn data() -> (Vec<f64>, Vec<i32>, Vec<Vec<f64>>) {
    let pt: Vec<f64> = (0..ENTRIES).map(|i| (i as f64 * 0.37).sin().abs() * 100.0).collect();
    let n: Vec<i32> = (0..ENTRIES).map(|i| i32::try_from(i % 17).unwrap()).collect();
    let jets: Vec<Vec<f64>> = (0..ENTRIES).map(|i| vec![i as f64; i % 5]).collect();
    (pt, n, jets)
}

fn bench(c: &mut Criterion) {
    let (pt, n, jets) = data();
    let batch = Batch::new()
        .column("pt", pt.clone())
        .unwrap()
        .column("n", n.clone())
        .unwrap()
        .column("jets", jets.clone())
        .unwrap();

    let mut g = c.benchmark_group("write");
    g.throughput(Throughput::Elements(ENTRIES as u64));
    g.sample_size(10);
    for format in [Format::Tree, Format::Ntuple] {
        g.bench_function(format!("{format:?}, DatasetWriter"), |b| {
            b.iter(|| {
                let mut w = DatasetWriter::new(Cursor::new(Vec::new()), "w.root", "events", format)
                    .compression(Compression::Zstd(1));
                w.write(&batch).unwrap();
                black_box(w.finish().unwrap())
            });
        });
    }
    g.bench_function("Tree, TreeWriter from rows", |b| {
        b.iter(|| {
            let mut w = TreeWriter::new(Cursor::new(Vec::new()), "w.root", "events", Compression::Zstd(1)).unwrap();
            w.write_batch(&[
                Branch::f64("pt", pt.clone()),
                Branch::i32("n", n.clone()),
                Branch::vector_f64("jets", jets.clone()),
            ])
            .unwrap();
            black_box(w.finish().unwrap())
        });
    });
    g.bench_function("Ntuple, NtupleWriter from rows", |b| {
        b.iter(|| {
            let mut w = NtupleWriter::new(Cursor::new(Vec::new()), "w.root", "events", Compression::Zstd(1)).unwrap();
            w.write_batch(&[
                Field::f64("pt", pt.clone()),
                Field::i32("n", n.clone()),
                Field::vector_f64("jets", jets.clone()),
            ])
            .unwrap();
            black_box(w.finish().unwrap())
        });
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
