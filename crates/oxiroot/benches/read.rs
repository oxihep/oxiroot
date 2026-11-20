//! Read throughput: a `TTree` and an RNTuple holding the same columns —
//! scalars and a `std::vector<double>` — written once under Zstd into a
//! temporary directory, then read back three ways: the format's own reader
//! (`values()`, one `Vec` per entry for a vector), its flat read, and the
//! `Dataset` API (flat columns, cluster by cluster). Plus reading histograms
//! out of a file holding many keys.
//!
//!   cargo bench -p oxiroot --bench read

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use oxiroot::ntuple::NtupleReader;
use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;

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
    let n: Vec<i32> = (0..ENTRIES)
        .map(|i| i32::try_from(i % 17).unwrap())
        .collect();
    let jets: Vec<Vec<f64>> = (0..ENTRIES).map(|i| vec![i as f64; i % 5]).collect();
    let batch = Batch::new()
        .column("pt", pt)
        .unwrap()
        .column("n", n)
        .unwrap()
        .column("jets", jets)
        .unwrap();

    let tree = dir.join("tree.root");
    let ntuple = dir.join("ntuple.root");
    for (path, format) in [(&tree, Format::Tree), (&ntuple, Format::Ntuple)] {
        let mut out = DatasetWriter::create(path, "events", format).unwrap();
        out.write(&batch).unwrap();
        out.finish().unwrap();
    }

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

    for (label, path) in [("ttree", &files.tree), ("rntuple", &files.ntuple)] {
        let mut g = c.benchmark_group(format!("read/{label}"));
        g.throughput(Throughput::Elements(ENTRIES as u64));
        g.sample_size(20);
        let file = FileReader::open(path).unwrap();

        g.bench_function("f64 column, low level", |b| {
            b.iter(|| match label {
                "ttree" => black_box(
                    TreeReader::open(&file, "events")
                        .unwrap()
                        .branch("pt")
                        .values()
                        .map(drop),
                ),
                _ => black_box(
                    NtupleReader::open(&file, "events")
                        .unwrap()
                        .field("pt")
                        .values()
                        .map(drop),
                ),
            });
        });
        g.bench_function("f64 column, Dataset::read", |b| {
            b.iter(|| {
                let ds = Dataset::open(&file, "events").unwrap();
                black_box(ds.read(["pt"]).unwrap())
            });
        });
        g.bench_function("vector<f64> column, low level values", |b| {
            b.iter(|| match label {
                "ttree" => black_box(
                    TreeReader::open(&file, "events")
                        .unwrap()
                        .branch("jets")
                        .values()
                        .map(drop),
                ),
                _ => black_box(
                    NtupleReader::open(&file, "events")
                        .unwrap()
                        .field("jets")
                        .values()
                        .map(drop),
                ),
            });
        });
        g.bench_function("vector<f64> column, low level flat", |b| {
            b.iter(|| match label {
                "ttree" => black_box(
                    TreeReader::open(&file, "events")
                        .unwrap()
                        .branch("jets")
                        .flat()
                        .map(drop),
                ),
                _ => black_box(
                    NtupleReader::open(&file, "events")
                        .unwrap()
                        .field("jets")
                        .flat()
                        .map(drop),
                ),
            });
        });
        g.bench_function("vector<f64> column, Dataset::read", |b| {
            b.iter(|| {
                let ds = Dataset::open(&file, "events").unwrap();
                black_box(ds.read(["jets"]).unwrap())
            });
        });
        g.bench_function("3 columns, Dataset::batches", |b| {
            b.iter(|| {
                let ds = Dataset::open(&file, "events").unwrap();
                let mut sum = 0.0;
                for batch in ds.batches(["pt", "n", "jets"]) {
                    let batch = batch.unwrap();
                    sum += batch.values::<f64>("pt").unwrap().iter().sum::<f64>();
                    sum += batch
                        .jagged::<f64>("jets")
                        .unwrap()
                        .values()
                        .iter()
                        .sum::<f64>();
                }
                black_box(sum)
            });
        });
        g.finish();
    }

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
