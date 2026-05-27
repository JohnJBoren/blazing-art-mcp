# Changelog

All notable changes to **blazing-art-mcp** are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-05-27

### Added

- **`tags.scm`-driven ingest engine.** `src/ingest.rs` now compiles a
  vendored tree-sitter `queries/<lang>/tags.scm` per language via a
  process-wide `OnceLock<RwLock<HashMap<Lang, Arc<Query>>>>` registry,
  replacing the v0.1 hand-coded `Lang::is_declaration` table and
  `Lang::extract_name` walker. Multi-pattern dedup by `(role, start_byte,
  end_byte)` with first-match-wins, which correctly classifies a Rust
  `function_item` inside `declaration_list` as `kind=method`, not
  `kind=function`.
- **Reference indexing.** A third key namespace
  `ref\x01<name>\x01<repo>\x01<path>:<line>` lives alongside `pri\x01...`
  and `sym\x01...`. `AstSymbol` gained a `role: SymbolRole` field
  (`Definition` | `Reference`, default `Definition` for backwards-compat
  JSON deserialization).
- **`findReferences(name, kind?, repo?, limit?)` MCP tool.** Single ART
  prefix scan over `ref\x01<name>\x01...`, with optional `repo` tightening
  the prefix and optional `kind` applied as a server-side filter
  (over-fetching by 4× to keep recall when filtering, capped at 10k).
- **`ingestStats(repo?)` MCP tool.** Returns total entry count,
  definition vs reference split, and a per-kind histogram, computed under
  a single read lock for consistency. Skips `sym\x01...` entries to avoid
  double-counting definitions.
- **5 new languages: Go, Java, C, C++, JavaScript.** Cargo deps:
  `tree-sitter-{go,java,c,cpp,javascript}`. Vendored upstream `tags.scm`
  with source SHAs noted in headers. C and C++ upstream tags shipped with
  zero `@reference.*` captures; both were augmented with `call_expression`
  and `type_identifier` patterns. Rust tags additionally augmented with
  `(scoped_identifier path: (identifier) @name) @reference.path` so
  `Memory::new`-style references resolve.
- **Parallel ingest via `rayon`.** `Memory::add_symbols_bulk` takes the
  write lock once for the whole batch; `ingest_repo` walks first, parses
  in parallel via `into_par_iter()`, then drains. Each rayon worker has
  its own `thread_local!` `HashMap<Lang, Parser>` pool.
- **Property + fuzz tests** (`tests/proptest_invariants.rs`):
  - Oracle equivalence: random insert/delete sequences against
    `BTreeMap<Vec<u8>, AstSymbol>` and `Memory.find_symbols` produce the
    same sorted multiset.
  - Key round-trip: encoded primary/inverted/ref keys recover the
    inserted symbol via inverted-prefix lookup.
  - Concurrent chaos: 4 std::thread workers + 1 reader thread, post-barrier
    consistency check.
- **Gold-set generator binary** `cargo run --bin build_goldset`. Walks
  `git log --no-merges`, filters short / boilerplate subjects, extracts
  identifier-like tokens, emits JSONL records when exactly one declaration
  in the index uniquely matches.
- **Hand-curated 30-record gold set** at `eval/goldset/handcurated.jsonl`
  covering five categories: ambiguous names, cross-language, refactor
  scenarios, long-tail languages, negative cases.
- **Eval runner binary** `cargo run --bin eval_goldset`. Computes
  Recall@{1, 5, 20}, MRR, p50/p99 latency, per-category breakdowns.
  Outputs JSON + Markdown to `eval/results/<sha>-<unix-ts>.{json,md}`.
- **CI threshold gate** `scripts/check_eval_threshold.sh`: reads the
  newest results file, compares to `eval/threshold.json`, exits non-zero
  if any floor (recall@5 ≥ 0.50, MRR ≥ 0.40, etc.) is violated.
- **Live integration harnesses** for Claude Code (`claude -p`) and
  kiro-cli (`kiro-cli chat --no-interactive --trust-all-tools`). Each runs
  10 scripted prompts under treatment (MCP enabled) vs control (no MCP),
  asserting via per-prompt regex pairs. Bash-3.2-portable; skip
  gracefully if their CLI isn't on PATH.
- **GitHub Actions CI** (`.github/workflows/ci.yml`): build + clippy +
  test on every push, eval gate on every push, harness jobs gated to
  `workflow_dispatch`.
- **Web demo extensions** (`static/index.html`): two new vanilla-JS
  panels for Find references + Ingest stats, with a CSS-only per-kind
  histogram. Quick-prefix buttons updated to v0.2 tags vocabulary.
- **Documentation:** `BENCHMARKS.md` v0.2 section with parallel-ingest
  numbers, retrieval-quality table, and harness reproducer commands.
  `README.md` updated with the 9-language list, `findReferences` /
  `ingestStats` rows in the tools table, kiro-cli + Codex CLI MCP config
  blocks, and a Tests section showing all three pyramid layers.
  `eval/README.md` documents the gold-set schema and folder layout.

### Changed

- **`AstSymbol.kind` vocabulary** switched from raw tree-sitter node
  kinds (`function_item`, `struct_item`, `trait_item`) to the standard
  tree-sitter "Code Navigation" tags vocabulary (`function`, `class`,
  `interface`, `module`, `macro`, `type`, `constant`). The schema and
  key shapes are unchanged. **Breaking** for any caller that previously
  built `sym\x01function_item\x01...` prefixes by hand; rewrite as
  `sym\x01function\x01...`. All v0.1 raw-node-name references in the
  README, web demo, and tests have been updated.

### Performance

Measured on Apple Silicon (Darwin 25.5.0, ARM64, `rustc 1.88.0`,
`profile = release`, `lto = true`, mimalloc):

| Target | Files | Indexed entries | Wall-clock cold ingest |
|---|---|---|---|
| own `src/` | 8 (.rs) | 748 (defs + refs) | **6.0 ms** |
| axum 0.8.9 | 60 (.rs) | (varies) | **28.0 ms** |

Hand-curated gold-set retrieval quality:

| Metric | Value |
|---|---|
| Recall@1 | **0.96** |
| Recall@5 | **1.00** |
| Recall@20 | **1.00** |
| MRR | **0.98** |
| Latency p50 / p99 | 0 / 4 μs |

### Deliberately deferred (v0.3 candidates)

- Persistence (snapshot + WAL).
- Live file-watcher + incremental re-ingest.
- Sharded `blart` for write concurrency at very large scale.
- Embedding-based retrieval comparison.
- LSIF / SCIP export.
- Additional tags.scm augmentation per language for higher reference recall.

## [0.1.0] - 2026-05-22

### Added

- Real ART backend via `blart::TreeMap<CString, T>` (replaces the v0.0
  `BTreeMap` placeholder).
- MCP server (hand-rolled JSON-RPC 2.0) over **stdio** + **Streamable HTTP**
  (axum 0.8, MCP spec 2025-06-18). Loopback-only binding,
  Origin-header validation, `/health` endpoint.
- Tree-sitter AST ingestion for Rust, Python, TypeScript, TSX (declarations
  only, hand-coded `Lang::is_declaration` table). MCP tools: `ingestRepo`,
  `findSymbols`, `deleteRepo`.
- Criterion benchmarks: point lookup, prefix scan, bulk insert, scaling
  sweep at 10k/100k/1M, real-data ingest tier (env-gated for cpython /
  TypeScript). Headline: ART prefix-scan ~570 ns flat through 1M keys
  (~100,000× faster than HashMap full-scan; ~3× faster than BTreeMap).
- Per-backend isolated RSS measurement (macOS-only). ART RSS at 1M keys is
  ~16% smaller than HashMap or BTreeMap.
- Vanilla-JS web demo at `static/index.html`.
- 13 tests (7 unit + 6 e2e against a Rust fixture).

[0.2.0]: https://github.com/JohnJBoren/blazing-art-mcp/releases/tag/v0.2.0
[0.1.0]: https://github.com/JohnJBoren/blazing-art-mcp/releases/tag/v0.1.0
