//! ART (`blart::TreeMap`) vs `BTreeMap` vs `HashMap` on a synthetic, AST-shaped key
//! workload — plus, in Phase 6, a scaling sweep across 10k → 1M (→ 5M when gated)
//! and a real-data ingest tier that exercises tree-sitter on actual repos.
//!
//! Why this shape? Realistic AST symbol keys share long prefixes
//! (`repo_03/src/auth/handler.rs:00042:003:fn:authenticate`). That is the
//! workload ART was designed for: long, prefix-shared, lexicographically
//! ordered keys with frequent prefix-range queries. Random UUIDs would
//! advantage hash tables and disadvantage ART, which is why we don't use them.
//!
//! Five Criterion groups:
//! - `point_lookup` — fetch one key chosen at random
//! - `prefix_scan_one_file` — fetch every key sharing a 2-segment prefix (~100 results)
//! - `insert_bulk_100k` — build the index from scratch
//! - `scaling_sweep` — same 3 ops at 10k / 100k / 1M (and 5M with `BLAZING_ART_BENCH_SCALE=full`)
//! - `real_data` — measure `ingest::ingest_repo` on this repo, CPython Lib (env var), TS compiler (env var)

#[path = "shared/keygen.rs"]
mod keygen;

use std::ffi::CString;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use blazing_art_mcp::{ingest, memory::Memory};

use keygen::{build_art, build_btree, build_hash, generate_keys_n, AstSymbol, XorShift};

const REPOS: usize = 10;
const FILES_PER_REPO: usize = 100;
const SYMBOLS_PER_FILE: usize = 100;

/// Legacy 100k generator — preserves the canonical Phase 2 dataset.
fn generate_keys() -> Vec<(String, AstSymbol)> {
    generate_keys_n(REPOS, FILES_PER_REPO, SYMBOLS_PER_FILE)
}

// ---------------------------------------------------------------------------
// Phase 2 benchmarks (preserved verbatim — same 100k baseline as in BENCHMARKS.md).
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_get_then_check)]
fn bench_point_lookup(c: &mut Criterion) {
    let keys = generate_keys();
    let art = build_art(&keys);
    let btree = build_btree(&keys);
    let hash = build_hash(&keys);

    let mut rng = XorShift::new(0xA5A5A5A5);
    let queries: Vec<String> = (0..1024)
        .map(|_| {
            let idx = (rng.next() as usize) % keys.len();
            keys[idx].0.clone()
        })
        .collect();

    let mut g = c.benchmark_group("point_lookup");
    g.throughput(Throughput::Elements(queries.len() as u64));

    g.bench_function("art", |b| {
        b.iter(|| {
            let mut hits = 0usize;
            for q in &queries {
                let ck = CString::new(q.as_bytes()).unwrap();
                if art.get(&ck).is_some() {
                    hits += 1;
                }
            }
            black_box(hits);
        });
    });

    g.bench_function("btreemap", |b| {
        b.iter(|| {
            let mut hits = 0usize;
            for q in &queries {
                if btree.get(q).is_some() {
                    hits += 1;
                }
            }
            black_box(hits);
        });
    });

    g.bench_function("hashmap", |b| {
        b.iter(|| {
            let mut hits = 0usize;
            for q in &queries {
                if hash.get(q).is_some() {
                    hits += 1;
                }
            }
            black_box(hits);
        });
    });

    g.finish();
}

fn bench_prefix_scan(c: &mut Criterion) {
    let keys = generate_keys();
    let art = build_art(&keys);
    let btree = build_btree(&keys);
    let hash = build_hash(&keys);

    let prefixes: Vec<String> = (0..8)
        .map(|i| format!("repo_{:02}\x01src/mod_{:03}/file_{:03}.rs\x01", i, i * 11, i * 11))
        .collect();

    let mut g = c.benchmark_group("prefix_scan_one_file");
    g.throughput(Throughput::Elements(prefixes.len() as u64));

    g.bench_function("art", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for p in &prefixes {
                total += art.prefix(p.as_bytes()).count();
            }
            black_box(total);
        });
    });

    g.bench_function("btreemap", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for p in &prefixes {
                total += btree
                    .range(p.clone()..)
                    .take_while(|(k, _)| k.starts_with(p.as_str()))
                    .count();
            }
            black_box(total);
        });
    });

    g.bench_function("hashmap_full_scan", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for p in &prefixes {
                total += hash.iter().filter(|(k, _)| k.starts_with(p.as_str())).count();
            }
            black_box(total);
        });
    });

    g.finish();
}

fn bench_insert_bulk(c: &mut Criterion) {
    let keys = generate_keys();

    let mut g = c.benchmark_group("insert_bulk_100k");
    g.throughput(Throughput::Elements(keys.len() as u64));
    g.sample_size(20);

    g.bench_with_input(BenchmarkId::new("art", keys.len()), &keys, |b, keys| {
        b.iter(|| {
            let m = build_art(keys);
            black_box(m.len());
        });
    });
    g.bench_with_input(BenchmarkId::new("btreemap", keys.len()), &keys, |b, keys| {
        b.iter(|| {
            let m = build_btree(keys);
            black_box(m.len());
        });
    });
    g.bench_with_input(BenchmarkId::new("hashmap", keys.len()), &keys, |b, keys| {
        b.iter(|| {
            let m = build_hash(keys);
            black_box(m.len());
        });
    });

    g.finish();
}

// ---------------------------------------------------------------------------
// Phase 6.2: scaling sweep — does ART's lead grow with N?
// ---------------------------------------------------------------------------

/// Tiers as `(repos, files_per_repo, syms_per_file, label)`.
fn scaling_tiers() -> Vec<(usize, usize, usize, &'static str)> {
    let full = std::env::var("BLAZING_ART_BENCH_SCALE")
        .map(|v| v == "full")
        .unwrap_or(false);

    let mut tiers: Vec<(usize, usize, usize, &str)> = vec![
        (10, 10, 100, "10k"),
        (10, 100, 100, "100k"),
        (10, 1000, 100, "1M"),
    ];
    if full {
        tiers.push((10, 5000, 100, "5M"));
    }
    tiers
}

fn bench_scaling_sweep(c: &mut Criterion) {
    let tiers = scaling_tiers();

    for (repos, files, syms, label) in &tiers {
        let n = repos * files * syms;
        eprintln!("=== scaling tier: {label} ({n} keys) ===");
        let keys = generate_keys_n(*repos, *files, *syms);

        // Use a small set of prefixes that match our generator's path pattern.
        // Each prefix returns ~syms keys.
        let prefixes: Vec<String> = (0..(*repos).min(4))
            .map(|i| format!("repo_{:02}\x01src/mod_{:03}/", i, 0))
            .collect();

        // Drop predecessor before allocating successor — keeps peak RSS bounded
        // when running the 5M tier.
        // ART
        {
            let art = build_art(&keys);
            let mut g = c.benchmark_group(format!("scaling_prefix_scan_{label}"));
            g.throughput(Throughput::Elements(prefixes.len() as u64));
            g.bench_function("art", |b| {
                b.iter(|| {
                    let mut total = 0usize;
                    for p in &prefixes {
                        total += art.prefix(p.as_bytes()).count();
                    }
                    black_box(total);
                });
            });
            g.finish();
        }

        // BTreeMap
        {
            let btree = build_btree(&keys);
            let mut g = c.benchmark_group(format!("scaling_prefix_scan_{label}"));
            g.throughput(Throughput::Elements(prefixes.len() as u64));
            g.bench_function("btreemap", |b| {
                b.iter(|| {
                    let mut total = 0usize;
                    for p in &prefixes {
                        total += btree
                            .range(p.clone()..)
                            .take_while(|(k, _)| k.starts_with(p.as_str()))
                            .count();
                    }
                    black_box(total);
                });
            });
            g.finish();
        }

        // HashMap (full scan, the only way without order)
        // Skip at the largest tier if not gated — full-scanning 5M keys per
        // sample is extremely slow and adds little signal beyond what 1M shows.
        if n <= 1_000_000 {
            let hash = build_hash(&keys);
            let mut g = c.benchmark_group(format!("scaling_prefix_scan_{label}"));
            g.throughput(Throughput::Elements(prefixes.len() as u64));
            g.sample_size(if n >= 1_000_000 { 10 } else { 30 });
            g.bench_function("hashmap_full_scan", |b| {
                b.iter(|| {
                    let mut total = 0usize;
                    for p in &prefixes {
                        total += hash.iter().filter(|(k, _)| k.starts_with(p.as_str())).count();
                    }
                    black_box(total);
                });
            });
            g.finish();
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 6.2: real-data ingest — exercises the full tree-sitter pipeline.
// Times wall-clock with `Instant`, not Criterion's repeated harness, because
// ingestion is not microsecond-sensitive and we want a single clean number.
// ---------------------------------------------------------------------------

fn time_ingest(label: &str, repo_id: &str, path: &Path) {
    if !path.exists() {
        eprintln!("real_data: SKIP {label} — path does not exist: {}", path.display());
        return;
    }
    let mem = Memory::new(1000);
    let t0 = Instant::now();
    let stats = ingest::ingest_repo(&mem, repo_id, path);
    let elapsed = t0.elapsed();
    let symbols_per_sec = stats.symbols_indexed as f64 / elapsed.as_secs_f64();
    eprintln!(
        "real_data: {label} | files={} symbols={} elapsed={:.2}s rate={:.0} sym/s errors={}",
        stats.files_parsed,
        stats.symbols_indexed,
        elapsed.as_secs_f64(),
        symbols_per_sec,
        stats.errors.len()
    );
}

fn bench_real_data(_c: &mut Criterion) {
    // Tier 1: this repo's own src/. Always runs.
    let own_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    time_ingest("blazing-art-mcp/src", "blazing-art", &own_src);

    // Tier 2: CPython Lib/. Runs if CPYTHON_LIB_PATH is set.
    if let Ok(p) = std::env::var("CPYTHON_LIB_PATH") {
        time_ingest("CPython Lib/", "cpython", &PathBuf::from(p));
    } else {
        eprintln!("real_data: CPython tier — set CPYTHON_LIB_PATH=/path/to/cpython/Lib to run");
    }

    // Tier 3: TypeScript compiler src/. Runs if TYPESCRIPT_SRC_PATH is set.
    if let Ok(p) = std::env::var("TYPESCRIPT_SRC_PATH") {
        time_ingest("TypeScript src/", "typescript", &PathBuf::from(p));
    } else {
        eprintln!("real_data: TypeScript tier — set TYPESCRIPT_SRC_PATH=/path/to/typescript/src to run");
    }
}

criterion_group!(
    benches,
    bench_point_lookup,
    bench_prefix_scan,
    bench_insert_bulk,
    bench_scaling_sweep,
    bench_real_data,
);
criterion_main!(benches);
