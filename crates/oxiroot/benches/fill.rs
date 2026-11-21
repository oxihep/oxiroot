//! Histogram fill throughput: ROOT histograms and oxihist histograms, serial
//! and across threads.
//!
//!   cargo bench -p oxiroot --bench fill

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxiroot::prelude::*;

/// Values the fills draw from: a deterministic spread over [0, 100), so the
/// benchmark needs no random number generator.
fn values(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 * 0.618_033_988_75) % 1.0 * 100.0)
        .collect()
}

const N: usize = 1_000_000;

fn root_hists(c: &mut Criterion) {
    let xs = values(N);
    let mut g = c.benchmark_group("fill/root");
    g.throughput(Throughput::Elements(N as u64));
    g.bench_function("Hist1D 100 bins", |b| {
        b.iter(|| {
            let mut h = Hist::reg(100, 0.0, 100.0).double();
            for &x in &xs {
                h.fill(x);
            }
            black_box(h)
        });
    });
    g.bench_function("Hist1D variable 100 bins", |b| {
        let edges: Vec<f64> = (0..=100).map(|i| f64::from(i).powf(1.5) / 10.0).collect();
        b.iter(|| {
            let mut h = Hist::var(&edges).double();
            for &x in &xs {
                h.fill(x);
            }
            black_box(h)
        });
    });
    g.bench_function("Hist2D 100x100", |b| {
        b.iter(|| {
            let mut h = Hist::reg(100, 0.0, 100.0).reg(100, 0.0, 100.0).double();
            for (&x, &y) in xs.iter().zip(xs.iter().rev()) {
                h.fill(x, y);
            }
            black_box(h)
        });
    });
    g.bench_function("Profile1D 100 bins", |b| {
        b.iter(|| {
            let mut p = Hist::reg(100, 0.0, 100.0).mean();
            for (&x, &y) in xs.iter().zip(xs.iter().rev()) {
                p.fill_sample(x, y);
            }
            black_box(p)
        });
    });
    g.finish();
}

fn oxihist_hists(c: &mut Criterion) {
    let xs = values(N);
    let mut g = c.benchmark_group("fill/oxihist");
    g.throughput(Throughput::Elements(N as u64));
    g.bench_function("regular 100 bins, Double", |b| {
        b.iter(|| {
            let mut h = Hist::reg(100, 0.0, 100.0).double();
            for &x in &xs {
                h.fill(x);
            }
            black_box(h)
        });
    });
    g.bench_function("regular 100 bins, Weight", |b| {
        b.iter(|| {
            let mut h = Hist::reg(100, 0.0, 100.0).weight();
            for &x in &xs {
                h.fill_weight(x, 0.5);
            }
            black_box(h)
        });
    });
    let cats: Vec<String> = (0..20).map(|i| format!("c{i}")).collect();
    g.bench_function("string category, 20 categories", |b| {
        b.iter(|| {
            let mut h = Hist::str_cat(&cats).double();
            for (i, _) in xs.iter().enumerate() {
                h.fill(cats[i % 20].as_str());
            }
            black_box(h)
        });
    });
    g.finish();
}

fn threaded(c: &mut Criterion) {
    let xs = values(N);
    let mut g = c.benchmark_group("fill/threaded");
    g.throughput(Throughput::Elements(N as u64));
    for threads in [1, 4] {
        g.bench_with_input(
            BenchmarkId::new("ThreadedHist<Histogram>", threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let acc = ThreadedHist::new(Hist::reg(100, 0.0, 100.0).double());
                    std::thread::scope(|s| {
                        for chunk in xs.chunks(N.div_ceil(threads)) {
                            let acc = &acc;
                            s.spawn(move || chunk.iter().for_each(|&x| acc.fill(x)));
                        }
                    });
                    black_box(acc.merge().unwrap())
                });
            },
        );
    }
    g.finish();
}

criterion_group!(benches, root_hists, oxihist_hists, threaded);
criterion_main!(benches);
