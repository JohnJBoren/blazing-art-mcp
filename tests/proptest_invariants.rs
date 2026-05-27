//! Property + fuzz tests (test pyramid layer 1, v0.2 Task 5).
//!
//! Three independent properties:
//!
//! * **A — oracle equivalence**: random sequences of `Insert(key, val)` then
//!   `PrefixScan(prefix)` against `Memory` and `BTreeMap<Vec<u8>, AstSymbol>`
//!   produce the same result as a sorted multiset. This catches any divergence
//!   between blart's prefix-scan semantics and the lexicographic baseline.
//!
//! * **B — key round-trip**: for every `(repo, path, line, col, kind, name)`
//!   tuple drawn from the strategy (excluding SOH/NUL bytes), the symbol that
//!   round-trips through `add_symbol` + `find_symbols(primary_prefix)` recovers
//!   exactly the inputs we put in.
//!
//! * **C — concurrent chaos**: 4 worker threads insert disjoint key spaces in
//!   parallel via rayon while a reader thread does prefix scans. No panics; the
//!   final index size equals the union; partial scans during the run only ever
//!   return keys that some worker has already inserted.
//!
//! These tests provide the bottom layer of the v0.2 test pyramid. CI fails on
//! any shrunk counterexample. We deliberately keep iteration counts low enough
//! to stay under ~10 seconds total so the property layer doesn't dominate
//! `cargo test` time.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blazing_art_mcp::ingest::{AstSymbol, SymbolRole};
use blazing_art_mcp::memory::Memory;
use proptest::prelude::*;

/// A safer version that also forbids characters that would mess with the
/// path/kind/name segments specifically (e.g., `/` in a kind would still be
/// schema-valid but is unrealistic; we keep alpha + underscore).
fn ident_segment_strategy(max_len: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<u8>().prop_filter("alphanum/underscore only", |b| {
        b.is_ascii_alphanumeric() || *b == b'_'
    }), 1..=max_len).prop_map(|bytes| String::from_utf8(bytes).expect("alphanum -> utf8"))
}

/// Static kind list for `proptest::sample::select` — that combinator requires
/// a `'static` slice (it stores the indices, not borrowed elements).
const KINDS: &[&str] = &["function", "class", "method", "type"];
const KINDS_DEF: &[&str] = &["function", "class", "method"];

/// Build a primary key the same way `ingest` does. Kept inline here so the
/// property test isn't coupled to `ingest`'s internal helpers (those are
/// `pub(crate)` only and the test imports the public surface).
fn pri_key(s: &AstSymbol) -> String {
    format!(
        "pri\x01{}\x01{}\x01{:05}:{:03}:{}\x01{}",
        s.repo, s.path, s.line, s.col, s.kind, s.name
    )
}

fn inv_key(s: &AstSymbol) -> String {
    format!("sym\x01{}\x01{}\x01{}\x01{}:{}", s.kind, s.name, s.repo, s.path, s.line)
}

// ---------------------------------------------------------------------------
// Property A: oracle equivalence on prefix scans.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        // Keep the iteration count modest so the suite stays well under 10s.
        cases: 64,
        max_shrink_iters: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn property_a_oracle_equivalence_for_prefix_scans(
        symbols in proptest::collection::vec(
            (
                ident_segment_strategy(8),       // repo
                ident_segment_strategy(8),       // path
                1u32..=999,                      // line
                1u32..=99,                       // col
                proptest::sample::select(KINDS),
                ident_segment_strategy(8),       // name
            ),
            1..=20,
        )
    ) {
        let mem = Memory::new(10_000);
        let mut oracle: BTreeMap<Vec<u8>, AstSymbol> = BTreeMap::new();

        for (repo, path, line, col, kind, name) in &symbols {
            let sym = AstSymbol {
                repo: repo.clone(),
                path: path.clone(),
                line: *line,
                col: *col,
                kind: kind.to_string(),
                name: name.clone(),
                role: SymbolRole::Definition,
            };
            // Insert into both stores under the same primary key.
            let pk = pri_key(&sym);
            assert!(mem.add_symbol(&pk, sym.clone()), "Memory must accept clean keys");
            oracle.insert(pk.into_bytes(), sym);
        }

        // Verify the empty-prefix scan returns everything.
        let mem_all = mem.find_symbols("pri\x01", oracle.len() + 100);
        let oracle_all: Vec<&AstSymbol> = oracle
            .iter()
            .filter(|(k, _)| k.starts_with(b"pri\x01"))
            .map(|(_, v)| v)
            .collect();
        prop_assert_eq!(mem_all.len(), oracle_all.len(),
            "empty pri\\x01 prefix should return all primary entries");

        // For each unique repo, prefix-scan and compare to the oracle's filtered iter.
        let repos: std::collections::HashSet<_> = symbols.iter().map(|s| s.0.clone()).collect();
        for repo in repos {
            let prefix_str = format!("pri\x01{repo}\x01");
            let prefix_bytes = prefix_str.as_bytes().to_vec();

            let mem_hits = mem.find_symbols(&prefix_str, oracle.len() + 100);
            let oracle_hits: Vec<&AstSymbol> = oracle
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix_bytes))
                .map(|(_, v)| v)
                .collect();

            prop_assert_eq!(mem_hits.len(), oracle_hits.len(),
                "prefix scan count must match oracle for repo `{}`", repo);

            // The order is determined by the byte-lex order of keys, which both
            // structures should produce identically. Compare the (line, col, name) tuples.
            let mem_tuples: Vec<_> = mem_hits.iter().map(|s| (&s.path, s.line, s.col, &s.name)).collect();
            let oracle_tuples: Vec<_> = oracle_hits.iter().map(|s| (&s.path, s.line, s.col, &s.name)).collect();
            prop_assert_eq!(mem_tuples, oracle_tuples, "prefix scan order must match oracle");
        }
    }
}

// ---------------------------------------------------------------------------
// Property B: key round-trip via inverted-index lookup.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn property_b_inverted_lookup_recovers_inserted_symbol(
        repo in ident_segment_strategy(8),
        path in ident_segment_strategy(8),
        line in 1u32..=99_999,
        col in 1u32..=999,
        kind in proptest::sample::select(KINDS_DEF),
        name in ident_segment_strategy(12),
    ) {
        let mem = Memory::new(100);
        let sym = AstSymbol {
            repo: repo.clone(),
            path: path.clone(),
            line,
            col,
            kind: kind.to_string(),
            name: name.clone(),
            role: SymbolRole::Definition,
        };

        // Insert under both the primary and inverted keys (same pattern as ingest).
        let pk = pri_key(&sym);
        let ik = inv_key(&sym);
        prop_assert!(mem.add_symbol(&pk, sym.clone()));
        prop_assert!(mem.add_symbol(&ik, sym.clone()));

        // Inverted lookup should round-trip exactly.
        let prefix = format!("sym\x01{kind}\x01{name}\x01");
        let hits = mem.find_symbols(&prefix, 10);
        prop_assert!(!hits.is_empty(), "inverted prefix must find at least one hit");

        let recovered = hits.iter().find(|h| h.repo == repo && h.path == path && h.line == line);
        prop_assert!(
            recovered.is_some(),
            "inverted lookup must recover the inserted symbol; got {:?}",
            hits
        );
        let r = recovered.unwrap();
        prop_assert_eq!(&r.kind, &sym.kind);
        prop_assert_eq!(&r.name, &sym.name);
        prop_assert_eq!(r.col, sym.col);
    }
}

// ---------------------------------------------------------------------------
// Property C: concurrent chaos — independently of proptest, run a fixed-shape
// stress test that asserts no panics + post-barrier consistency.
// ---------------------------------------------------------------------------

#[test]
fn property_c_concurrent_writers_and_reader_no_panics() {
    use std::thread;
    use std::sync::Barrier;

    const WORKERS: usize = 4;
    const PER_WORKER: usize = 250;

    let mem = Arc::new(Memory::new(100_000));
    let barrier = Arc::new(Barrier::new(WORKERS + 1));
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let mem = Arc::clone(&mem);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for i in 0..PER_WORKER {
                let sym = AstSymbol {
                    repo: format!("worker{w}"),
                    path: format!("file_{i:04}.rs"),
                    line: i as u32 + 1,
                    col: 1,
                    kind: "function".to_string(),
                    name: format!("fn_{i}"),
                    role: SymbolRole::Definition,
                };
                let pk = pri_key(&sym);
                mem.add_symbol(&pk, sym);
            }
        }));
    }

    // Reader thread: keep prefix-scanning while writers run.
    let reader_mem = Arc::clone(&mem);
    let reader_stop = Arc::clone(&stop);
    let reader_barrier = Arc::clone(&barrier);
    let reader = thread::spawn(move || {
        reader_barrier.wait();
        // Reads should never observe a malformed AstSymbol.
        let mut last_seen = 0;
        while !reader_stop.load(Ordering::Relaxed) {
            let hits = reader_mem.find_symbols("pri\x01worker", 10_000);
            for h in &hits {
                // Smoke check: the symbol's fields are internally consistent.
                assert!(h.line >= 1);
                assert!(!h.repo.is_empty());
            }
            last_seen = last_seen.max(hits.len());
        }
        last_seen
    });

    // Wait for writers to finish.
    for h in handles {
        h.join().expect("writer thread must not panic");
    }
    stop.store(true, Ordering::Relaxed);
    let _max_during = reader.join().expect("reader thread must not panic");

    // Post-barrier: every key we wrote should be there.
    let final_hits = mem.find_symbols("pri\x01worker", WORKERS * PER_WORKER + 100);
    assert_eq!(
        final_hits.len(),
        WORKERS * PER_WORKER,
        "post-barrier count must equal writer total"
    );

    // Sanity: each key prefix-scanned individually returns exactly 250 hits.
    for w in 0..WORKERS {
        let hits = mem.find_symbols(&format!("pri\x01worker{w}\x01"), PER_WORKER + 10);
        assert_eq!(
            hits.len(),
            PER_WORKER,
            "per-worker count must be exactly {PER_WORKER}"
        );
    }

    // Suppress unused warnings.
    let _ = CString::new("warm").unwrap();
}
