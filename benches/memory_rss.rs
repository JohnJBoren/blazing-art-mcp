//! RSS memory measurement at 1M keys: ART vs BTreeMap vs HashMap.
//!
//! `harness = false` because Criterion's repeated execution defeats one-shot
//! resident-set-size measurement. This bench has its own `main`.
//!
//! macOS-only by design. On macOS, `ru_maxrss` is in bytes. On Linux it's
//! kilobytes. Without per-platform handling, a Linux run would silently
//! publish numbers 1024× too small. Keep this guard until someone properly
//! ports the Linux path.
//!
//! Run with:
//!   cargo bench --bench memory_rss -- --nocapture

#[cfg(not(target_os = "macos"))]
compile_error!(
    "memory_rss bench is macOS-only because ru_maxrss units differ across platforms \
     (bytes on macOS, KB on Linux, etc). Add a target-specific code path before enabling this elsewhere."
);

#[path = "shared/keygen.rs"]
mod keygen;

use std::mem::MaybeUninit;

use keygen::{build_art, build_btree, build_hash, generate_keys_n};

/// macOS: `ru_maxrss` is the maximum resident set size in BYTES.
fn current_max_rss_bytes() -> u64 {
    let mut usage: MaybeUninit<libc::rusage> = MaybeUninit::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    assert_eq!(rc, 0, "getrusage failed");
    let usage = unsafe { usage.assume_init() };
    usage.ru_maxrss as u64
}

fn fmt_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
}

fn main() {
    // 1M keys (10 repos × 1000 files × 100 symbols).
    const REPOS: usize = 10;
    const FILES: usize = 1000;
    const SYMS: usize = 100;
    let n = REPOS * FILES * SYMS;

    let target = std::env::var("BLAZING_ART_RSS_TARGET").ok();

    println!("# memory_rss bench — {n} synthetic AST-shaped keys");
    println!("# macOS: ru_maxrss is in bytes. Reported as MB for readability.");
    println!();

    if target.is_none() {
        println!("# Running ALL backends in one process (cumulative high-watermark mode).");
        println!("# For clean per-backend isolated numbers, run three times:");
        println!("#   BLAZING_ART_RSS_TARGET=art      cargo bench --bench memory_rss -- --nocapture");
        println!("#   BLAZING_ART_RSS_TARGET=btreemap cargo bench --bench memory_rss -- --nocapture");
        println!("#   BLAZING_ART_RSS_TARGET=hashmap  cargo bench --bench memory_rss -- --nocapture");
        println!();
    } else {
        println!("# ISOLATED MODE: BLAZING_ART_RSS_TARGET={}", target.as_deref().unwrap());
        println!("# Each backend in its own process gives clean peak-RSS numbers.");
        println!();
    }

    let keys = generate_keys_n(REPOS, FILES, SYMS);
    let baseline = current_max_rss_bytes();
    println!(
        "Generated {} keys. RSS after key generation: {}",
        keys.len(),
        fmt_mb(baseline)
    );
    println!();
    println!("| Backend     | Length     | RSS peak after build | Delta from baseline |");
    println!("|-------------|------------|----------------------|---------------------|");

    let want = |name: &str| target.as_deref().is_none() || target.as_deref() == Some(name);

    if want("art") {
        let m = build_art(&keys);
        let after = current_max_rss_bytes();
        println!(
            "| art         | {:>10} | {:>20} | {:>19} |",
            m.len(),
            fmt_mb(after),
            fmt_mb(after.saturating_sub(baseline))
        );
        drop(m);
    }
    if want("btreemap") {
        let m = build_btree(&keys);
        let after = current_max_rss_bytes();
        println!(
            "| btreemap    | {:>10} | {:>20} | {:>19} |",
            m.len(),
            fmt_mb(after),
            fmt_mb(after.saturating_sub(baseline))
        );
        drop(m);
    }
    if want("hashmap") {
        let m = build_hash(&keys);
        let after = current_max_rss_bytes();
        println!(
            "| hashmap     | {:>10} | {:>20} | {:>19} |",
            m.len(),
            fmt_mb(after),
            fmt_mb(after.saturating_sub(baseline))
        );
        drop(m);
    }

    println!();
    println!("# Methodology note:");
    println!("# `ru_maxrss` is a HIGH WATERMARK over the process lifetime. When all three");
    println!("# backends run sequentially in one process, the second and third see lower");
    println!("# DELTAS because the watermark already includes the first backend's pages.");
    println!("# Use BLAZING_ART_RSS_TARGET=<backend> to measure each in isolation.");
}
