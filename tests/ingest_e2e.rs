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
fn dual_key_invariant_def_pri_equals_def_inv() {
    // Each @definition.* match writes BOTH a pri\x01... primary key and a
    // sym\x01... inverted key. Each @reference.* match writes ONE ref\x01...
    // entry. So the dual-key invariant applies only to the definition side:
    // pri count == inv count, and total = 2*pri + ref.
    let mem = fresh_memory();
    let stats = ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    let pri = mem.find_symbols(&format!("pri\x01{REPO_ID}\x01"), 1000);
    let inv = mem.find_symbols("sym\x01", 1000);
    let refs = mem.find_symbols("ref\x01", 1000);

    assert_eq!(pri.len(), inv.len(), "every definition writes both pri and inv");
    assert_eq!(
        stats.symbols_indexed,
        pri.len() + inv.len() + refs.len(),
        "stats.symbols_indexed must equal the sum across all three namespaces"
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
        // Tags vocabulary: top-level fn → kind="function" (was "function_item"
        // before the tags.scm refactor; see CLAUDE.md "Docs vs. Reality").
        assert_eq!(s.kind, "function");
    }
}

#[test]
fn inverted_lookup_finds_function_by_name_across_files() {
    let mem = fresh_memory();
    ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    // Tags vocabulary: top-level fn → kind="function".
    let hits = mem.find_symbols("sym\x01function\x01add\x01", 100);
    assert_eq!(hits.len(), 1, "exactly one fn named `add` in the fixture");
    let s = &hits[0];
    assert_eq!(s.name, "add");
    assert_eq!(s.kind, "function");
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
    let refs_gone = mem.find_symbols("ref\x01", 10);
    assert!(refs_gone.is_empty(), "delete_repo_symbols must also clear ref\\x01 entries");
}

#[test]
fn references_are_indexed_under_ref_namespace() {
    use blazing_art_mcp::ingest::SymbolRole;

    let mem = fresh_memory();
    ingest::ingest_repo(&mem, REPO_ID, &fixture_path());

    // The fixture has no function calls (no call_expression nodes), so the
    // only reference upstream tree-sitter-rust tags.scm picks up is
    // `impl Circle` → @reference.implementation capturing `Circle`.
    let circle_refs = mem.find_symbols("ref\x01Circle\x01", 100);
    assert!(
        !circle_refs.is_empty(),
        "expected at least one reference to `Circle` from the fixture's impl block"
    );
    for r in &circle_refs {
        assert_eq!(r.role, SymbolRole::Reference, "ref\\x01... entries must have role=Reference");
        assert_eq!(r.name, "Circle");
        assert_eq!(r.repo, REPO_ID);
        assert_eq!(r.kind, "implementation");
    }

    // Inverted-def lookup still works for the definition side.
    let circle_def = mem.find_symbols("sym\x01class\x01Circle\x01", 10);
    assert_eq!(circle_def.len(), 1);
    assert_eq!(circle_def[0].role, SymbolRole::Definition);
    assert_eq!(circle_def[0].kind, "class");
}

#[test]
fn ingest_multilang_fixture() {
    use blazing_art_mcp::ingest::SymbolRole;

    const ML_REPO: &str = "multilang";
    let mem = fresh_memory();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-multilang");
    let stats = ingest::ingest_repo(&mem, ML_REPO, &path);

    assert!(stats.errors.is_empty(), "multilang fixture must parse cleanly: {:?}", stats.errors);
    assert_eq!(stats.files_parsed, 5, "fixture has exactly 5 files (one per language)");

    // Definitions: 3 Greeter.java + 3 box.cpp + 3 counter.js + 3 main.go + 4 point.c = 16
    let defs = mem.find_symbols(&format!("pri\x01{ML_REPO}\x01"), 1000);
    assert_eq!(defs.len(), 16, "expected exactly 16 definitions across the 5 fixture files");
    for d in &defs {
        assert_eq!(d.role, SymbolRole::Definition);
    }

    // References: at least one per file. Exact counts depend on grammar coverage,
    // but each language should produce at least 3 refs from the fixture.
    let refs = mem.find_symbols("ref\x01", 1000);
    assert!(refs.len() >= 15, "expected at least 15 references; got {}", refs.len());
    for r in &refs {
        assert_eq!(r.role, SymbolRole::Reference);
    }

    // Per-language smoke: at least one canonical decl per language is findable
    // via the inverted index.
    let greeter = mem.find_symbols("sym\x01class\x01Greeter\x01", 10);
    assert_eq!(greeter.len(), 1, "Java class Greeter must be indexed");

    let counter = mem.find_symbols("sym\x01class\x01Counter\x01", 10);
    assert_eq!(counter.len(), 1, "JS class Counter must be indexed");

    let box_cls = mem.find_symbols("sym\x01class\x01Box\x01", 10);
    assert_eq!(box_cls.len(), 1, "C++ class Box must be indexed");

    // C struct Point — surfaced as kind=class in tags vocabulary.
    let point = mem.find_symbols("sym\x01class\x01Point\x01", 10);
    assert_eq!(point.len(), 1, "C struct Point must be indexed (as kind=class)");

    // Go type Vec3 — surfaced as kind=type via @definition.type.
    let vec3 = mem.find_symbols("sym\x01type\x01Vec3\x01", 10);
    assert_eq!(vec3.len(), 1, "Go type Vec3 must be indexed (as kind=type)");

    // Cross-language: there are TWO `main` functions in the fixture
    // (Java method main, Go function main, C function main = three actually).
    // findSymbols on sym\x01function\x01main\x01 picks up the C and Go ones;
    // the Java one is kind=method.
    let main_fns = mem.find_symbols("sym\x01function\x01main\x01", 10);
    assert_eq!(main_fns.len(), 2, "C main + Go main = 2 function-kind 'main' decls");
    let main_methods = mem.find_symbols("sym\x01method\x01main\x01", 10);
    assert_eq!(main_methods.len(), 1, "Java main is method-kind");
}
