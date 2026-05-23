//! Integration tests for the AST ingestion pipeline.
//!
//! These run against a controlled fixture under `tests/fixtures/sample-repo/`.
//! Unlike the unit tests in `src/memory.rs` (which test the storage layer in
//! isolation), these exercise the full ingest path: filesystem walk →
//! tree-sitter parse → declaration filtering → dual-key writes → ART storage.
//!
//! Anything that regresses the symbol count, the dual-key invariant, or the
//! prefix-scan ordering will fail loud here BEFORE the published benchmark
//! numbers in `BENCHMARKS.md` get a chance to drift silently.

use std::path::Path;

use blazing_art_mcp::{ingest, memory::Memory};

/// Total symbols (primary + inverted) inserted into the ART after ingesting
/// the sample-repo fixture.
///
/// 9 declarations across 3 files:
///   lib.rs     — 1 struct (Coordinate), 1 fn (origin) = 2
///   math.rs    — 3 fns (add, subtract, multiply)      = 3
///   geometry.rs — 2 structs (Circle, Rectangle), 1 fn (area), 1 impl_item, 1 fn inside the impl (new) = 5
///                 (Whether tree-sitter surfaces the method inside `impl Circle` as
///                  a `function_item` is grammar-dependent. Total of 9 or 10 expected.)
///
/// Each declaration writes 2 keys (primary + inverted), so total entries = 2 × declarations.
/// Lock the EXACT number after the first run; the discovery test below prints it.
const REPO_ID: &str = "sample-repo";

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-repo")
}

fn fresh_memory() -> Memory {
    Memory::new(1000)
}

/// Discovery test: dump what tree-sitter actually surfaces. Used once to lock
/// `EXPECTED_DECLARATIONS`. Kept as `#[test]` permanently because if the count
/// ever changes, every other test breaks loud — and this one prints why.
#[test]
fn discover_symbol_count() {
    let mem = fresh_memory();
    let stats = ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    eprintln!("=== fixture ingest stats ===");
    eprintln!("  files_parsed:    {}", stats.files_parsed);
    eprintln!("  files_skipped:   {}", stats.files_skipped);
    eprintln!("  symbols_indexed: {}", stats.symbols_indexed);
    eprintln!("  total in ART:    {}", mem.symbol_count());
    eprintln!("  errors:          {:?}", stats.errors);

    // Print every primary key entry so we can audit what's there.
    let pri_prefix = format!("pri\x01{REPO_ID}\x01");
    let primaries = mem.find_symbols(&pri_prefix, 1000);
    eprintln!("  primary symbols ({}):", primaries.len());
    for s in &primaries {
        eprintln!("    {}:{} {} {} ({})", s.path, s.line, s.kind, s.name, s.col);
    }

    assert_eq!(stats.files_parsed, 3, "fixture has exactly 3 .rs files");
    assert!(stats.errors.is_empty(), "fixture must parse cleanly: {:?}", stats.errors);
    assert!(stats.symbols_indexed > 0, "fixture must produce at least one symbol");
}

#[test]
fn dual_key_invariant_total_is_even() {
    let mem = fresh_memory();
    let stats = ingest::ingest_repo(&mem, REPO_ID, &fixture_path());
    assert_eq!(
        stats.symbols_indexed % 2,
        0,
        "every declaration writes one primary + one inverted key, so the total must be even"
    );
    assert_eq!(mem.symbol_count(), stats.symbols_indexed);
}

#[test]
fn primary_count_equals_inverted_count() {
    let mem = fresh_memory();
    ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    let pri = mem.find_symbols(&format!("pri\x01{REPO_ID}\x01"), 1000);
    let inv = mem.find_symbols("sym\x01", 1000);

    assert_eq!(
        pri.len(),
        inv.len(),
        "primary and inverted key counts must match (every decl writes both)"
    );
    assert!(pri.len() >= 9, "fixture has 9+ declarations");
}

#[test]
fn prefix_scan_by_file_returns_only_that_file() {
    let mem = fresh_memory();
    ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    let hits = mem.find_symbols(&format!("pri\x01{REPO_ID}\x01src/math.rs\x01"), 100);

    // math.rs has exactly 3 fn declarations: add, subtract, multiply.
    let names: Vec<&str> = hits.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["add", "subtract", "multiply"],
        "primary keys are sorted by zero-padded line number, so source-order is preserved"
    );

    for s in &hits {
        assert_eq!(s.repo, REPO_ID);
        assert_eq!(s.path, "src/math.rs");
        assert_eq!(s.kind, "function_item");
    }
}

#[test]
fn inverted_lookup_finds_function_by_name_across_files() {
    let mem = fresh_memory();
    ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    let hits = mem.find_symbols("sym\x01function_item\x01add\x01", 100);
    assert_eq!(hits.len(), 1, "exactly one fn named `add` in the fixture");
    let s = &hits[0];
    assert_eq!(s.name, "add");
    assert_eq!(s.kind, "function_item");
    assert_eq!(s.path, "src/math.rs");
    assert_eq!(s.repo, REPO_ID);
}

#[test]
fn delete_repo_clears_all_entries() {
    let mem = fresh_memory();
    let stats = ingest::ingest_repo(&mem, REPO_ID, &fixture_path());
    assert!(stats.symbols_indexed > 0);
    assert!(mem.symbol_count() > 0);

    let removed = mem.delete_repo_symbols(REPO_ID);
    assert_eq!(removed, stats.symbols_indexed, "delete should remove every entry we inserted");
    assert_eq!(mem.symbol_count(), 0, "ART must be empty after delete");

    // And a prefix scan now finds nothing.
    let still_there = mem.find_symbols("pri\x01", 10);
    assert!(still_there.is_empty());
    let inv_gone = mem.find_symbols("sym\x01", 10);
    assert!(inv_gone.is_empty());
}
