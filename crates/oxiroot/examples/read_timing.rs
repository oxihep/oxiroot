//! Time `Dataset` reads of a `TTree` or `RNTuple`, for the comparison with
//! uproot in `scripts/bench_uproot.py`.
//!
//! ```sh
//! # write the synthetic files: tree.root and ntuple.root, `n` entries each
//! cargo run --release -p oxiroot --features rayon --example read_timing -- write DIR N
//! # time reading COLUMNS (comma-separated) of NAME in FILE, REPEATS times
//! cargo run --release -p oxiroot --features rayon --example read_timing -- read FILE NAME COLUMNS REPEATS
//! ```
//!
//! `read` prints one JSON object: the median wall time in milliseconds of a
//! serial read (`Dataset::read`) and, with the `rayon` feature, of a parallel
//! one (`batches(..).parallel(true)`, columns and clusters on rayon's pool).
//! Every timing opens the file afresh, as uproot's does.

use std::path::Path;
use std::time::Instant;

use oxiroot::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("write") if args.len() == 3 => write(Path::new(&args[1]), args[2].parse()?),
        Some("read") if args.len() == 5 => {
            let columns: Vec<&str> = args[3].split(',').collect();
            read(Path::new(&args[1]), &args[2], &columns, args[4].parse()?)
        }
        _ => {
            eprintln!(
                "usage: read_timing write DIR N\n       read_timing read FILE NAME COLUMNS REPEATS"
            );
            std::process::exit(2);
        }
    }
}

/// `tree.root` and `ntuple.root` in `dir`, each `events` with `n` entries:
/// `pt` (f64), `n` (i32) and `jets` (`std::vector<double>`, 0 to 4 values).
fn write(dir: &Path, n: usize) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let pt: Vec<f64> = (0..n)
        .map(|i| (i as f64 * 0.37).sin().abs() * 100.0)
        .collect();
    let count: Vec<i32> = (0..n).map(|i| i32::try_from(i % 17).unwrap()).collect();
    let mut offsets = Vec::with_capacity(n + 1);
    let mut values = Vec::new();
    offsets.push(0_u64);
    for i in 0..n {
        for j in 0..i % 5 {
            values.push((i * 5 + j) as f64 * 0.01);
        }
        offsets.push(values.len() as u64);
    }
    let batch = Batch::new()
        .column("pt", pt)?
        .column("n", count)?
        .list("jets", offsets, values)?;
    for (file, format) in [("tree.root", Format::Tree), ("ntuple.root", Format::Ntuple)] {
        FileWriter::create(dir.join(file))
            .add(NamedBatch::new("events", format, batch.clone()))
            .write()?;
    }
    Ok(())
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn time(
    repeats: usize,
    mut f: impl FnMut() -> Result<(), Box<dyn std::error::Error>>,
) -> Result<f64, Box<dyn std::error::Error>> {
    f()?; // warm the page cache and the allocator
    let mut times = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        f()?;
        times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    Ok(median(times))
}

fn read(
    path: &Path,
    name: &str,
    columns: &[&str],
    repeats: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let serial = time(repeats, || {
        let file = FileReader::open(path)?;
        let batch = Dataset::open(&file, name)?.read(columns)?;
        std::hint::black_box(batch);
        Ok(())
    })?;
    #[cfg(feature = "rayon")]
    let parallel = {
        let ms = time(repeats, || {
            let file = FileReader::open(path)?;
            let ds = Dataset::open(&file, name)?;
            let mut all = Batch::new();
            for batch in ds.batches(columns).step(ds.entries()).parallel(true) {
                all.append(batch?)?;
            }
            std::hint::black_box(all);
            Ok(())
        })?;
        format!("{ms:.3}")
    };
    #[cfg(not(feature = "rayon"))]
    let parallel = "null";
    println!("{{\"serial_ms\": {serial:.3}, \"parallel_ms\": {parallel}}}");
    Ok(())
}
