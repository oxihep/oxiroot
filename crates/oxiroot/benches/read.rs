//! Read throughput: TTree branches and an RNTuple, written once under Zstd
//! into a temporary directory, then read back; plus reading histograms out of
//! a file holding many keys.
//!
//!   cargo bench -p oxiroot --bench read

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use oxiroot::prelude::*;

const ENTRIES: usize = 1_000_000;

struct Files {
    dir: PathBuf,
    tree: PathBuf,
    ntuple: PathBuf,
    hists: PathBuf,
}

impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn write_files() -> Files {
    let dir = std::env::temp_dir().join(format!("oxiroot-bench-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pt: Vec<f64> = (0..ENTRIES)
        .map(|i| (i as f64 * 0.37).sin().abs() * 100.0)
        .collect();
    let n: Vec<i32> = (0..ENTRIES).map(|i| (i % 17) as i32).collect();
    let jets: Vec<Vec<f64>> = (0..ENTRIES / 10).map(|i| vec![i as f64; i % 5]).collect();

    let tree = dir.join("tree.root");
    write_tree_file(
        &tree,
        "events",
        &[Branch::f64("pt", pt.clone()), Branch::i32("n", n.clone())],
        Compression::Zstd(5),
    )
    .unwrap();
    write_tree_file(
        dir.join("jagged.root"),
        "events",
        &[Branch::jagged_f64("jets", jets)],
        Compression::Zstd(5),
    )
    .unwrap();

    let ntuple = dir.join("ntuple.root");
    write_rntuple_file(
        &ntuple,
        "ntpl",
        &[Field::f64("pt", pt), Field::i32("n", n)],
        Compression::Zstd(5),
    )
    .unwrap();

    let hists = dir.join("hists.root");
    let mut w = FileWriter::create(&hists);
    let made: Vec<Hist1D> = (0..500)
        .map(|i| {
            let mut h = Hist::reg(100, 0.0, 1.0)
                .double()
                .to_root()
                .named(format!("h{i}"));
            h.fill(0.5);
            h
        })
        .collect();
    for h in &made {
        w = w.add(h);
    }
    w.write(Compression::Zstd(5)).unwrap();

    Files {
        dir,
        tree,
        ntuple,
        hists,
    }
}

fn bench(c: &mut Criterion) {
    let files = write_files();

    let mut g = c.benchmark_group("read/ttree");
    g.throughput(Throughput::Elements(ENTRIES as u64));
    g.sample_size(20);
    g.bench_function("f64 branch, 1M entries", |b| {
        b.iter(|| {
            let file = FileReader::open(&files.tree).unwrap();
            let tree = TreeReader::open(&file, "events").unwrap();
            black_box(tree.read_branch(&file, "pt").unwrap())
        });
    });
    g.bench_function("i32 branch, 1M entries", |b| {
        b.iter(|| {
            let file = FileReader::open(&files.tree).unwrap();
            let tree = TreeReader::open(&file, "events").unwrap();
            black_box(tree.read_branch(&file, "n").unwrap())
        });
    });
    g.finish();

    let mut g = c.benchmark_group("read/ttree-jagged");
    g.throughput(Throughput::Elements((ENTRIES / 10) as u64));
    g.sample_size(20);
    g.bench_function("jagged f64 branch, 100k entries", |b| {
        let path = files.dir.join("jagged.root");
        b.iter(|| {
            let file = FileReader::open(&path).unwrap();
            let tree = TreeReader::open(&file, "events").unwrap();
            black_box(tree.read_branch(&file, "jets").unwrap())
        });
    });
    g.finish();

    let mut g = c.benchmark_group("read/rntuple");
    g.throughput(Throughput::Elements(ENTRIES as u64));
    g.sample_size(20);
    g.bench_function("f64 + i32 fields, 1M entries", |b| {
        b.iter(|| {
            let file = FileReader::open(&files.ntuple).unwrap();
            let ntuple = NtupleReader::open(&file, "ntpl").unwrap();
            black_box(ntuple.read_field(&file, "pt").unwrap());
            black_box(ntuple.read_field(&file, "n").unwrap())
        });
    });
    g.finish();

    let mut g = c.benchmark_group("read/keys");
    g.throughput(Throughput::Elements(500));
    g.bench_function("500 TH1D by name", |b| {
        b.iter(|| {
            let file = FileReader::open(&files.hists).unwrap();
            for i in 0..500 {
                black_box(Hist1D::read_root(&file, &format!("h{i}")).unwrap());
            }
        });
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
