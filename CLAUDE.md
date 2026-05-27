# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Important: Docs vs. Reality

**The README, Makefile, and shell scripts describe many features that are not implemented in the actual binary.** Treat them as aspirational. Before changing or referencing a feature, verify it exists in `src/simple_mcp.rs` (the only source file).

What the docs claim vs. what the code does:

| Claim (README/Makefile/CLAUDE history) | Reality in `src/simple_mcp.rs` |
|---|---|
| Backed by Adaptive Radix Tree (`art-tree`) | **DONE 2026-05-22.** Now uses `blart::TreeMap<CString, T>` (real ART) in `src/memory.rs`. Keys are `CString` because `blart::insert` requires `NoPrefixesBytes` (variable `String` doesn't satisfy it; `CString`'s NUL terminator does). Prefix scans use `TreeMap::prefix(&[u8])`. **Segment separator inside composite keys is `\x01` (SOH), NOT `\x00`** — see Phase 4 schema. |
| Zero-copy `rkyv` serialization | Standard `serde_json` |
| `rmcp` official MCP SDK | Hand-rolled JSON-RPC 2.0 (works over both stdio and HTTP) |
| WebSocket transport (`--ws`) | **Not WebSocket, but the related goal is met:** `--http <addr>` runs the MCP Streamable HTTP transport (axum 0.8) per spec 2025-06-18. `POST /mcp` for JSON-RPC, `GET /mcp` returns 405 (no proactive messages), `GET /health` for liveness, `GET /` serves `static/index.html` if present. Origin header validated; bind restricted to loopback. |
| Health endpoints / Axum / `--health-port` | `--http` enables Axum + `/health`. No separate `--health-port`. |
| OpenTelemetry / `--telemetry` flag | Not implemented; `eprintln!` only |
| `--health-check` flag | Not implemented |
| Tests (`cargo test`, `tests/integration`) | **7 unit tests** in `src/memory.rs` `#[cfg(test)] mod tests`. No `tests/` integration dir yet. End-to-end demo at `/tmp/blazing_art_demo.py` (Python urllib script that ingests src/, prefix-scans by file, does inverted-index lookup, deletes). |
| AST ingestion (Phase 4) | **DONE.** `src/ingest.rs` parses .rs/.py/.ts/.tsx **plus .go/.java/.c+.h/.cc+.cpp+.cxx+.hpp+.hh+.hxx/.js+.mjs+.cjs+.jsx (v0.2 Task 3, 2026-05)** — 9 grammars total. Indexes every declaration symbol under both a primary key (`pri\x01<repo>\x01<path>\x01<line5>:<col3>:<kind>\x01<name>`) and an inverted key (`sym\x01<kind>\x01<name>\x01<repo>\x01<path>:<line>`). **v0.2 Task 1:** the engine is driven by vendored tree-sitter `tags.scm` queries under `queries/<lang>/tags.scm`, replacing the hand-coded `is_declaration` table + `extract_name` walker. Side effect: the `kind` field stored on each `AstSymbol` uses the *tags vocabulary* (`function`, `class`, `method`, `interface`, `module`, `macro`, `type`, `constant`) instead of raw tree-sitter node kinds. Per-language Query is compiled once via an `OnceLock<RwLock<HashMap<Lang, Arc<Query>>>>` registry. **v0.2 Task 2:** `@reference.*` captures are also indexed, into a third namespace `ref\x01<name>\x01<repo>\x01<path>:<line>`. Each `AstSymbol` now has a `role: SymbolRole` field (`Definition`/`Reference`, default `Definition` for back-compat). New MCP tool `findReferences(name, kind?, repo?, limit?)`. Vendored Rust tags.scm augmented with `scoped_identifier path` capture (`Memory::new`-style refs). C and C++ tags.scm augmented with `call_expression`, `field_expression`, and bare `type_identifier` ref captures (upstream is decl-only). New multilang fixture at `tests/fixtures/sample-multilang/{Greeter.java, main.go, counter.js, box.cpp, point.c}` produces 16 defs + 23 refs across the 5 new languages. **v0.2 Task 4:** `ingest_repo` is now parallelized via `rayon::par_iter` over the file walker. Each rayon worker uses a `thread_local!` `HashMap<Lang, Parser>` pool; shared `Arc<Query>` per language is fetched lazily. New `Memory::add_symbols_bulk` takes the write lock once for the whole batch. New benchmark `bench_parallel_ingest` group in `benches/art_benchmarks.rs`. Measured wall-clock on M-series Mac: own src (8 .rs files, 748 entries) ~6 ms; axum-0.8.9 (60 .rs files) ~28 ms. Definition dual-key invariant still holds: pri count == inv count; total = 2×def + ref. |
| Web demo (Phase 5) | **DONE.** `static/index.html` — single-file vanilla-JS demo, no framework. Served at `GET /` by the HTTP transport. Three panels: ingest, prefix search, results. Quick-prefix buttons demonstrate the schema. README rewritten with Claude Code MCP config blocks for both stdio and HTTP transports. |
| Publication-ready benchmarks (Phase 6) | **DONE.** Refactor: shared keygen at `benches/shared/keygen.rs`. New benches: scaling sweep (`bench_scaling_sweep` at 10k/100k/1M/5M*; gated by `BLAZING_ART_BENCH_SCALE=full`), real-data ingest (`bench_real_data` over own src + CPython Lib + TS compiler, env-var-skipped), and standalone `benches/memory_rss.rs` (`harness=false`, macOS-only via `compile_error!()` on other targets, isolated per-backend mode via `BLAZING_ART_RSS_TARGET=art\|btreemap\|hashmap`). Integration tests at `tests/ingest_e2e.rs` (6 tests, fixture under `tests/fixtures/sample-repo/`). Total tests: 13. `BENCHMARKS.md` adds Scaling, Real-data ingestion, Memory (RSS), Reproducing sections. Headlines: ART prefix-scan ~constant ~570 ns from 10k→1M (HashMap full-scan grows linearly to 58.8 ms = **~100,000× gap at 1M**); ART RSS at 1M is 187 MB vs HashMap 223 MB / BTreeMap 224 MB (**ART ~16% smaller**). |
| Benchmarks (`cargo bench`) | `benches/art_benchmarks.rs` exists. Phase 2 results published in `BENCHMARKS.md` (ART prefix-scan ~3× BTreeMap, ~5,700× HashMap full-scan; insert competitive with HashMap). |
| Entry point `src/main.rs` | **DONE.** Module split: `src/lib.rs` (declarations), `src/main.rs` (CLI + transport dispatch), `src/memory.rs` (ART-backed `Memory`), `src/protocol.rs` (JSON-RPC dispatch), `src/transport/{stdio,http}.rs`. |

`run-mcp.sh`, the Makefile's `run-ws`/`health-check`/`bench` targets, and most of the README will fail or be silently ignored. If the user asks for those features, the work is to *build* them, not to invoke them.

## Build, Run, Test

```bash
cargo build --release                                   # Binary: target/release/blazing_art_mcp
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all

# Run (STDIO MCP server — speaks JSON-RPC on stdin/stdout)
cargo run --release -- --entities data/entities.json --events data/events.json

# Smoke test the JSON-RPC surface (initialize, tools/list, tools/call)
./test-mcp.sh                                           # requires release build
```

Single-line manual probe:
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | ./target/release/blazing_art_mcp --entities data/entities.json
```

There is no test suite. `cargo test` succeeds because there are zero tests — don't interpret a green run as validation.

## Architecture

Single-file MCP server in `src/simple_mcp.rs` (~490 lines). It speaks **MCP over stdio only** using hand-written JSON-RPC 2.0 handling.

**Data model** — two in-memory stores, both `Arc<RwLock<BTreeMap<String, T>>>`:
- `Entity { name, summary, born?, tags }` keyed by name
- `Event { id, timestamp, description, category }` keyed by id

`find_events(prefix)` is a `BTreeMap::range(..).take_while(starts_with)` prefix scan capped by `--event-limit` (default 100 in code, 64 in docs — code wins). This is the closest the project gets to its "ART" branding.

**Request loop** (`main` → `handle_request`):
1. Read a line from stdin → parse as `JsonRpcRequest`.
2. Dispatch on `method`: `initialize`, `tools/list`, `tools/call`, plus the `notifications/initialized` notification (no response).
3. For `tools/call`, branch on `params.name` across four tools: `lookupEntity`, `addEntity`, `findEvents`, `addEvent`. Tool results are wrapped as `{ content: [{ type: "text", text: <stringified-json> }] }` per MCP convention.
4. Write response + `\n` + flush. Broken-pipe errors break the loop cleanly (Claude Desktop closes stdio on shutdown).

**Logging discipline:** all diagnostics go to `eprintln!` (stderr). Stdout is reserved for JSON-RPC frames — anything written there corrupts the protocol. `mcp-wrapper.sh` exists specifically to redirect stderr to `mcp-server.log` when running under a host that conflates the streams.

**Allocator:** `mimalloc::MiMalloc` is set as `#[global_allocator]`. On musl targets, `Cargo.toml` swaps in `tikv-jemallocator` via a `cfg(target_env = "musl")` dependency.

**Release profile** is aggressive: `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. Expect long release builds.

## CLI Surface

The binary accepts only three flags. Anything else passed (e.g. `--ws`, `--health-port`, `--telemetry`, `--health-check`) will fail clap parsing.

```
--entities <FILE>      Optional JSON file of Entity records to preload
--events <FILE>        Optional JSON file of Event records to preload
--event-limit <NUM>    Cap for findEvents prefix scan (default 100)
```

Sample data lives in both `examples/` and `data/` (different content; `data/` has a richer set used by `test-mcp.sh`).

## When extending this project

- The binary path is `src/simple_mcp.rs` (declared in `Cargo.toml [[bin]]`). A new `src/main.rs` will be ignored unless you also update `Cargo.toml`.
- To actually deliver on the project's "ART" name, the swap point is the `Memory` struct — replace `BTreeMap<String, T>` with an ART implementation while keeping the `lookup_entity` / `find_events` signatures.
- To add WebSocket/health endpoints (the README's claimed features), there's no scaffolding to extend — you'd be adding `axum` / `tokio-tungstenite` and the corresponding CLI flags from scratch.
- Python files (`extract_papers.py`, `enhance_extraction.py`) are unrelated data-prep scripts that pull from a local Qdrant instance to generate `entities.json` / `events.json`. They're not part of the server runtime.
