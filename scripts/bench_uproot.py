#!/usr/bin/env python3
"""Read the same TTree and RNTuple files with oxiroot's `Dataset` and with
uproot, and print the median times as a Markdown table.

    python3 scripts/bench_uproot.py [--entries N] [--repeats R] [--out DIR]

Files: the synthetic `events` (pt f64, n i32, jets std::vector<double>)
written by oxiroot (`read_timing write`) and, when `root` is on PATH, by ROOT's
RDataFrame (`scripts/bench_write_root.C`), all with Zstd level 5. Each read
opens the file afresh and reads every entry of the columns into memory: a
`Batch` for oxiroot, NumPy / Awkward arrays for uproot. Both run warm, from
the page cache, after one untimed read.

Serial is one thread on each side. Parallel is oxiroot's
`batches(..).parallel(true)` on rayon's pool against uproot with a
`ThreadPoolExecutor` of as many threads for decompression and interpretation.

Needs uproot and awkward (the repository's `.venv`, see scripts/interop_local.sh).
"""

import argparse
import concurrent.futures
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

import uproot

ROOT_DIR = Path(__file__).resolve().parent.parent
CASES = [("pt", ["pt"]), ("pt, n, jets", ["pt", "n", "jets"])]


def build():
    subprocess.run(
        ["cargo", "build", "--release", "-q", "-p", "oxiroot", "--features", "rayon",
         "--example", "read_timing"],
        cwd=ROOT_DIR, check=True)
    return ROOT_DIR / "target" / "release" / "examples" / "read_timing"


def oxiroot_times(exe, path, columns, repeats):
    out = subprocess.run(
        [str(exe), "read", str(path), "events", ",".join(columns), str(repeats)],
        check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def median_ms(repeats, f):
    f()
    times = []
    for _ in range(repeats):
        start = time.perf_counter()
        f()
        times.append((time.perf_counter() - start) * 1e3)
    return statistics.median(times)


def uproot_times(path, columns, repeats, threads):
    def serial():
        with uproot.open(path) as f:
            f["events"].arrays(columns)

    def parallel():
        with concurrent.futures.ThreadPoolExecutor(threads) as pool:
            with uproot.open(path, decompression_executor=pool,
                             interpretation_executor=pool) as f:
                f["events"].arrays(columns, decompression_executor=pool,
                                   interpretation_executor=pool)

    return {"serial_ms": median_ms(repeats, serial),
            "parallel_ms": median_ms(repeats, parallel)}


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--entries", type=int, default=10_000_000)
    p.add_argument("--repeats", type=int, default=5)
    p.add_argument("--out", type=Path, default=ROOT_DIR / "target" / "bench-uproot")
    args = p.parse_args()
    threads = os.cpu_count() or 1

    exe = build()
    args.out.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(exe), "write", str(args.out), str(args.entries)], check=True)
    files = [("oxiroot", "TTree", args.out / "tree.root"),
             ("oxiroot", "RNTuple", args.out / "ntuple.root")]
    if shutil.which("root"):
        macro = ROOT_DIR / "scripts" / "bench_write_root.C"
        subprocess.run(["root", "-l", "-b", "-q",
                        f'{macro}("{args.out}", {args.entries})'],
                       check=True, capture_output=True)
        files += [("ROOT", "TTree", args.out / "root_tree.root"),
                  ("ROOT", "RNTuple", args.out / "root_ntuple.root")]

    rows = []
    for writer, fmt, path in files:
        for label, columns in CASES:
            ox = oxiroot_times(exe, path, columns, args.repeats)
            up = uproot_times(path, columns, args.repeats, threads)
            rows.append({"writer": writer, "format": fmt, "columns": label,
                         "mib": path.stat().st_size / 2**20,
                         "oxiroot": ox, "uproot": up})
            print(f"{writer} {fmt} [{label}]: oxiroot {ox}, uproot {up}",
                  file=sys.stderr)

    (args.out / "results.json").write_text(json.dumps({
        "entries": args.entries, "repeats": args.repeats, "threads": threads,
        "machine": platform.platform(), "uproot": uproot.__version__,
        "rows": rows}, indent=2))

    print(f"{args.entries:,} entries, median of {args.repeats}, "
          f"{threads} threads, uproot {uproot.__version__}\n")
    print("| File | Columns | oxiroot | uproot | Speed-up | "
          "oxiroot parallel | uproot parallel | Speed-up |")
    print("|---|---|---:|---:|---:|---:|---:|---:|")
    for r in rows:
        o, u = r["oxiroot"], r["uproot"]
        par = (f"{o['parallel_ms']:.0f} ms | {u['parallel_ms']:.0f} ms | "
               f"{u['parallel_ms'] / o['parallel_ms']:.1f}×"
               if o["parallel_ms"] is not None else "— | — | —")
        print(f"| {r['writer']} {r['format']} ({r['mib']:.0f} MiB) | {r['columns']} | "
              f"{o['serial_ms']:.0f} ms | {u['serial_ms']:.0f} ms | "
              f"{u['serial_ms'] / o['serial_ms']:.1f}× | {par} |")


if __name__ == "__main__":
    main()
