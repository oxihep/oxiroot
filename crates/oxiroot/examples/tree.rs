//! Write a `TTree` and read it back. Run:
//!
//! ```sh
//! cargo run -p oxiroot --example tree
//! ```
//!
//! Writes a scalar, a variable-length list and a string column in batches with
//! [`DatasetWriter`] (it closes a cluster every ~32 MiB, so memory stays
//! bounded), then reads them back with [`Dataset`]: all at once, one cluster at
//! a time, and an entry window that touches only the baskets it needs. The same
//! code writes an `RNTuple` with `Format::Ntuple`. Pass an output path as the
//! first argument to keep the file.

use oxiroot::prelude::*;

fn main() -> oxiroot::Result<()> {
    let keep = std::env::args().nth(1);
    let path = keep.clone().unwrap_or_else(|| {
        std::env::temp_dir()
            .join("oxiroot_tree.root")
            .display()
            .to_string()
    });

    // Five batches of 1000 events: a scalar, a list (rows of differing
    // length), and a string.
    let mut out = DatasetWriter::create(&path, "Events", Format::Tree)?;
    for chunk in 0..5_i32 {
        let event: Vec<i32> = (chunk * 1000..(chunk + 1) * 1000).collect();
        let hits: Vec<Vec<f64>> = event
            .iter()
            .map(|&i| vec![f64::from(i); usize::try_from(i % 3).unwrap_or(0)])
            .collect();
        let label: Vec<String> = event.iter().map(|i| format!("e{i}")).collect();
        let batch = Batch::new()
            .column("event", event)?
            .column("hits", hits)?
            .column("label", label)?;
        out.append(batch)?;
    }
    println!("wrote {} entries to {path}", out.entries());
    out.finish()?;

    let file = FileReader::open(&path)?;
    let events = Dataset::open(&file, "Events")?;
    println!(
        "{:?} with {} entries, columns {:?}",
        events.format(),
        events.entries(),
        events.columns()
    );

    // Everything at once: typed slices, no copies.
    let all = events.read(["event", "hits", "label"])?;
    let hits = all.jagged::<f64>("hits")?;
    println!(
        "event[..5] = {:?}, hits[2] = {:?}, label[4] = {:?}",
        &all.values::<i32>("event")?[..5],
        hits.row(2),
        all.strings("label")?.get(4)
    );

    // One cluster at a time: the memory a loop needs is one batch.
    let mut total = 0.0;
    for batch in events.batches(["hits"]) {
        let batch = batch?;
        total += batch.jagged::<f64>("hits")?.values().iter().sum::<f64>();
    }
    println!("sum of all hits = {total}");

    // A window: only the baskets covering [2500, 2505) are read.
    let window = events
        .batches(["event"])
        .range(2500..2505)
        .next()
        .transpose()?;
    if let Some(w) = window {
        println!("event[2500..2505] = {:?}", w.values::<i32>("event")?);
    }

    // The TTree reader underneath, for what only a tree has.
    if let Some(tree) = events.as_tree() {
        for b in tree.branch_names() {
            println!(
                "  {b}: {:?}, {:?}",
                tree.branch_type(b).unwrap(),
                tree.branch_layout(b).unwrap()
            );
        }
    }

    if keep.is_none() {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}
