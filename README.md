# blazing-art-mcp

> An Adaptive Radix Tree (Leis et al. 2013) wired up as a Model Context Protocol
> server. Indexes tree-sitter ASTs of any codebase into microsecond-latency,
> prefix-scannable structured memory for coding agents.

The mission: make ART a defensible primitive for agentic memory by making it
*real* and *measurably better than the alternatives* on the workload coding
agents actually run — prefix scans over symbol-shaped keys.

## What's actually here

| Component | Where |
|---|---|
| Real ART backend (`blart::TreeMap`) | `src/memory.rs` |
| JSON-RPC dispatch | `src/protocol.rs` |
| Stdio MCP transport | `src/transport/stdio.rs` |
| HTTP+SSE MCP transport (axum, spec 2025-06-18) | `src/transport/http.rs` |
| Tree-sitter AST ingestion (Rust, Python, TypeScript, TSX, **Go, Java, C, C++, JavaScript** — v0.2) | `src/ingest.rs`, `queries/<lang>/tags.scm` |
| Criterion benchmarks (point/prefix/insert + scaling sweep + real-data) | `benches/art_benchmarks.rs` |
| Per-backend isolated RSS measurement (macOS-only) | `benches/memory_rss.rs` |
| Shared synthetic key generator | `benches/shared/keygen.rs` |
| Integration tests against a 3-file fixture | `tests/ingest_e2e.rs` |
| Headline numbers + methodology | `BENCHMARKS.md` |
| Vanilla-JS web demo | `static/index.html` |

## Tools exposed

| Tool | What it does |
|---|---|
| `lookupEntity(name)` | Exact-match entity fetch from in-memory store. |
| `addEntity(name, summary, born?, tags)` | Insert/update an entity. |
| `findEvents(prefix)` | ART prefix scan over event ids (capped by `--event-limit`). |
| `addEvent(id?, timestamp?, description, category)` | Insert/update an event. |
| `ingestRepo(path, repo_id?)` | Walk a repo, parse `.rs/.py/.ts/.tsx/.go/.java/.c/.cpp/.js` with tree-sitter, index every declaration **and reference** symbol via the `tags.scm`-driven engine (v0.2). Each declaration writes a primary + inverted key; each reference writes a `ref\x01...` key. Parse stage runs in parallel via `rayon`. |
| `findSymbols(prefix, limit?)` | ART prefix scan over the symbol index. See key schema below. |
| `findReferences(name, kind?, repo?, limit?)` | **(v0.2)** Find every call-site / use of a symbol by name across the index. Backed by a single prefix scan over the `ref\x01<name>\x01...` namespace; `kind` and `repo` further tighten the filter server-side. |
| `deleteRepo(repo_id)` | Remove every symbol entry for a repo (definitions and references). |

## Key schema (locked — don't change without re-checking `blart::NoPrefixesBytes`)

The segment separator inside composite keys is **`\x01` (SOH)**, *not* `\x00`,
because `blart::TreeMap::insert` requires `NoPrefixesBytes`, which `String`
doesn't satisfy. We use `CString::new(<bytes-with-internal-SOH-but-no-NUL>)`,
which appends the trailing `\x00` that makes any inserted key prefix-free.

```
Primary  : pri\x01<repo>\x01<rel_path>\x01<line5>:<col3>:<kind>\x01<name>
Inverted : sym\x01<kind>\x01<name>\x01<repo>\x01<rel_path>:<line>
```

Prefix scan examples:

| Prefix | Returns |
|---|---|
| `pri\x01myrepo\x01` | every symbol in the repo (sorted by file, then line) |
| `pri\x01myrepo\x01src/auth.rs\x01` | every symbol in that file (sorted by line) |
| `pri\x01myrepo\x01src/\x01` | every symbol in `src/` |
| `sym\x01function_item\x01parse_request\x01` | every `parse_request` function across all repos |
| `sym\x01struct_item\x01Memory\x01` | every `Memory` struct across all repos |

## Build and run

```bash
cargo build --release                    # binary: target/release/blazing_art_mcp
cargo test                               # 13 tests (7 unit + 6 integration)
cargo clippy --all-targets -- -D warnings

# Default Criterion suite (point lookup, prefix scan, bulk insert,
# scaling sweep at 10k/100k/1M, real-data tier 1 = own src/):
cargo bench --bench art_benchmarks

# Add the 5M scaling tier (requires ~2.5 GB peak RSS):
BLAZING_ART_BENCH_SCALE=full cargo bench --bench art_benchmarks

# Real-data tiers 2 + 3 (require local repos):
CPYTHON_LIB_PATH=$HOME/cpython/Lib       cargo bench --bench art_benchmarks
TYPESCRIPT_SRC_PATH=$HOME/typescript/src cargo bench --bench art_benchmarks

# Per-backend isolated RSS measurement (run all three for the table in BENCHMARKS.md):
BLAZING_ART_RSS_TARGET=art      cargo bench --bench memory_rss -- --nocapture
BLAZING_ART_RSS_TARGET=btreemap cargo bench --bench memory_rss -- --nocapture
BLAZING_ART_RSS_TARGET=hashmap  cargo bench --bench memory_rss -- --nocapture
```

### Stdio transport (default — for Claude Code / Claude Desktop)

```bash
./target/release/blazing_art_mcp \
  --entities data/entities.json \
  --events data/events.json
```

### HTTP+SSE transport (for browsers and HTTP-MCP clients)

```bash
./target/release/blazing_art_mcp --http 127.0.0.1:4242
# Then visit http://127.0.0.1:4242/ for the demo UI,
# or POST JSON-RPC to http://127.0.0.1:4242/mcp.
# /health returns {"status":"ok"}.
```

The `--http` flag is restricted to loopback addresses by design, per MCP spec
guidance for local servers (defends against DNS rebinding). The handler also
validates the `Origin` header.

### Smoke test

```bash
./test-mcp.sh                            # exercises all four legacy tools over stdio
python3 /tmp/blazing_art_demo.py         # exercises ingestRepo / findSymbols / deleteRepo over HTTP
                                         # (requires server running with --http 127.0.0.1:4242)
```

## Hooking it up to a coding agent

### Claude Code (stdio)

Add to `~/.claude/claude_code_config.json`:

```json
{
  "mcpServers": {
    "blazing-art": {
      "command": "/abs/path/to/blazing-art-mcp/target/release/blazing_art_mcp",
      "args": [
        "--entities", "/abs/path/to/blazing-art-mcp/data/entities.json",
        "--events",   "/abs/path/to/blazing-art-mcp/data/events.json"
      ]
    }
  }
}
```

Then in any Claude Code session, the agent can call `ingestRepo` on a path,
followed by `findSymbols` with prefix scans — see the schema table above.

### Claude Code (HTTP)

Run the server once with `--http 127.0.0.1:4242`, then in
`~/.claude/claude_code_config.json`:

```json
{
  "mcpServers": {
    "blazing-art": {
      "transport": "http",
      "url": "http://127.0.0.1:4242/mcp"
    }
  }
}
```

The HTTP path lets you share one server process across multiple agent sessions.

### kiro-cli (stdio)

```bash
kiro-cli mcp add \
    --name blazing-art \
    --scope workspace \
    --command /abs/path/to/blazing-art-mcp/target/release/blazing_art_mcp \
    --args "--entities,/abs/path/to/data/entities.json,--events,/abs/path/to/data/events.json"

# Optional but recommended for slow first-ingest:
kiro-cli settings mcp.noInteractiveTimeout 60000
```

Then in any kiro-cli chat session, the agent has `ingestRepo`, `findSymbols`,
`findReferences`, and the rest. To remove later: `kiro-cli mcp remove --name blazing-art`.

### Codex CLI (stdio)

OpenAI's Codex CLI supports MCP servers over stdio only (no remote HTTP). Add
this to `~/.codex/config.toml`:

```toml
[mcp_servers.blazing_art]
command = "/abs/path/to/blazing-art-mcp/target/release/blazing_art_mcp"
args = ["--entities", "/abs/path/to/data/entities.json",
        "--events",   "/abs/path/to/data/events.json"]
```

### Browser-based chatbots

Open `http://127.0.0.1:4242/` for the bundled demo. Or POST JSON-RPC straight
from your own JS — see `static/index.html` for a 200-line vanilla-JS reference.

```js
async function lookup(name) {
  const r = await fetch("http://127.0.0.1:4242/mcp", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0", id: 1, method: "tools/call",
      params: { name: "lookupEntity", arguments: { name } },
    }),
  });
  return r.json();
}
```

## Why ART for agent memory? (the proof)

`BENCHMARKS.md` has the full methodology, raw Criterion output, and honest
caveats. The shortest version of the story is two charts:

**Prefix-scan latency vs N** — the workload coding agents actually run:

| N (keys) | ART | BTreeMap | HashMap full-scan | ART/BTree | ART vs HashMap |
|---|---|---|---|---|---|
| 10k | 547 ns | 1,487 ns | 221 µs | 2.7× faster | **404× faster** |
| 100k | 569 ns | 1,519 ns | 3,297 µs | 2.7× faster | **5,795× faster** |
| 1M | **573 ns** | **1,448 ns** | **58,800 µs** | 2.5× faster | **102,605× faster** |

ART and BTreeMap stay roughly constant as N grows. **HashMap full-scan
grows linearly** — the only way to "prefix scan" a hash table is iterate
every key and string-match. By 1M keys, the gap is **5 orders of
magnitude**. The choice of backing index matters *more* as the dataset
grows, exactly the regime agentic codebase indexing is heading toward.

**RSS at 1M keys** — measured per-backend in isolated processes (because
`getrusage` returns a process-lifetime high watermark, not current):

| Backend | Peak RSS delta | vs ART |
|---|---|---|
| **ART** (`blart::TreeMap`) | **187 MB** | **smallest** |
| HashMap | 223 MB | +19% |
| BTreeMap | 225 MB | +20% |

ART is ~16% smaller than either alternative — path compression stores
shared key prefixes once per branch instead of redundantly per key.

**What ART is not:** ART loses point-lookup to HashMap by ~5× at 100k
(148 µs vs 32 µs). ART is not a hash-table replacement. It's the right
primitive when prefix queries are part of the workload — which is
exactly what coding agents doing structured codebase navigation produce.

## Tests (three layers — v0.2)

```bash
# Layer 1 — unit + e2e + property/fuzz (fast; runs on every push).
cargo test --all-targets
#   ↳ 30 tests:
#      15 unit (memory + ingest)
#       8 e2e (sample-repo + multilang fixtures)
#       3 proptest (oracle vs BTreeMap, key round-trip, concurrent chaos)
#       4 handcurated_schema (gold-set sanity)

# Layer 2 — gold-set retrieval eval (runs on every push; gates merges).
cargo run --release --bin eval_goldset
bash scripts/check_eval_threshold.sh
#   ↳ Recall@1 / @5 / @20, MRR, p50/p99, per-category breakdowns;
#     fails if any metric below floors in eval/threshold.json.

# Layer 3 — live coding-agent harnesses (manual; cost real LLM API calls).
bash tests/harness/run_claude_code.sh    # skips if `claude` not installed
bash tests/harness/run_kiro_cli.sh       # skips if `kiro-cli` not installed
#   ↳ Drives each agent through 10 scripted prompts under
#     treatment (MCP enabled) vs control (no MCP); regex assertions
#     in tests/harness/asserts.txt require treatment to match the
#     correct answer AND control to MISS it.
```

CI workflow: see `.github/workflows/ci.yml`. Layers 1 + 2 run on every push and
PR; layer 3 is gated to `workflow_dispatch` because it costs real API credits.

## Repository layout

```
src/
├── lib.rs                       # module declarations
├── main.rs                      # CLI + transport dispatch
├── memory.rs                    # ART-backed Memory (entities, events, symbols) + 7 unit tests
├── protocol.rs                  # JSON-RPC 2.0 dispatch (transport-agnostic)
├── ingest.rs                    # tree-sitter walker + key encoder
└── transport/
    ├── mod.rs
    ├── stdio.rs                 # newline-delimited JSON-RPC over stdin/stdout
    └── http.rs                  # axum: POST /mcp, GET /mcp (405), GET /health, GET /
benches/
├── art_benchmarks.rs            # Criterion: point/prefix/insert + scaling sweep + real-data
├── memory_rss.rs                # standalone (harness=false), macOS-only via getrusage
└── shared/keygen.rs             # synthetic key generator + index builders, shared by both
tests/
├── ingest_e2e.rs                # 6 integration tests against the fixture
└── fixtures/sample-repo/src/    # 3-file Rust fixture (10 known declarations)
static/index.html                # vanilla-JS web demo (no framework)
data/                            # 2,758 entities + 896 events (AI/ML papers from Qdrant)
examples/                        # smaller sample dataset
BENCHMARKS.md                    # publication-ready numbers + methodology
CLAUDE.md                        # docs-vs-reality table for future sessions
```

## Honest limits

- **In-memory only.** No persistence yet. Restart = empty index. Re-ingest from source.
- **Single-threaded writes.** `Arc<RwLock<TreeMap>>`. No concurrent-write story.
  (Swap to `congee` for lock-free ART-OLC if multi-writer becomes a real workload.)
- **No auth/TLS.** Bound to loopback. Don't expose this to the internet.
- **Declaration nodes only.** Identifiers, expressions, statements aren't indexed —
  only function/struct/class/trait/type definitions. Saves ~100× memory at the
  cost of no "find every reference" capability (use an LSP for that).
- **No incremental re-ingestion.** `ingestRepo` does a full re-parse. For
  large repos this is a few seconds; for monorepos it could take minutes.
- **macOS-only RSS bench.** `getrusage(ru_maxrss)` units differ by platform —
  bytes on macOS, kilobytes on Linux. The harness `compile_error!`s on
  non-macOS to prevent silent 1024× errors. Latency benches run anywhere.

## v0.1 milestone

The five `/goal` success criteria are met with reproducible evidence:

1. ✅ `BTreeMap` gone, real `blart::TreeMap` is the index, clippy clean, 13 tests
2. ✅ `ingestRepo` <500 ms / `findSymbols` <1 ms (own `src/` ingests in 18 ms; prefix scan ~570 ns)
3. ✅ Stdio + HTTP+SSE transports both green against the same `Memory`
4. ✅ ART prefix-scan ≥1.5× over `BTreeMap` and uses less RSS than `HashMap` (~16% smaller at 1M)
5. ✅ ART exact lookup faster than embeddings (sub-µs vs typical 50ms+ — the math is in `BENCHMARKS.md`)

**Natural follow-ups for v0.2** (deliberately out of scope for v0.1): persistence
(snapshot on shutdown / WAL), `congee` for concurrent writes, Go/Java/C grammars,
LSIF compatibility, side-by-side embeddings benchmark.

## License

MIT — see [LICENSE](LICENSE).
