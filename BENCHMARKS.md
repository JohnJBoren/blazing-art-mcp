# BENCHMARKS

`blart::TreeMap` (ART) vs `std::collections::BTreeMap` vs
`std::collections::HashMap` on a synthetic AST-shaped key workload.

The original Phase 2 numbers (100k keys) are below in the **Results**
section unchanged. Phase 6 added the **Scaling**, **Real-data**, and
**Memory (RSS)** sections at the bottom.

## Workload

100,000 keys generated deterministically:

- 10 repos × 100 files × 100 symbols
- Key shape: `repo_NN\x01src/mod_NNN/file_NNN.rs\x01LLLLL:CCC:KIND\x01name_N`
  where `\x01` is the segment separator (SOH — chosen because file paths
  and symbol names cannot contain it, and because `CString::new` accepts
  it as an interior byte while still adding the trailing `\x00` that
  satisfies blart's `NoPrefixesBytes` bound).
- Value: a small `AstSymbol { line, col, kind, name }` struct (cloneable).

Each map is built once with all 100k keys; the benchmark then exercises
queries over the populated structure. Three operations are measured:

1. **`point_lookup`** — fetch a single key from a sample of 1,024 random
   queries; one iteration runs all 1,024 lookups.
2. **`prefix_scan_one_file`** — for 8 different one-file prefixes, count
   how many keys match. Each prefix returns ~100 keys (one file's worth
   of symbols).
3. **`insert_bulk_100k`** — build the entire structure from scratch.

Hardware: Apple Silicon (Darwin 25.5.0, ARM64). Compiler: `rustc 1.88.0`,
`profile = release`, `lto = true`, `codegen-units = 1`,
`mimalloc` global allocator. Criterion 0.5, 100 samples per group except
inserts (20 samples — 100k inserts per iteration is heavy).

Reproduce: `cargo bench --bench art_benchmarks`.

## Results

### Point lookup (1024 queries per iteration)

| Backend | Median time | Throughput | vs HashMap | vs BTreeMap |
|---|---|---|---|---|
| `HashMap` | 32.21 µs | 31.79 M elem/s | **1.00× (winner)** | 6.62× faster |
| `blart::TreeMap` (ART) | 147.97 µs | 6.92 M elem/s | 0.22× | **1.44× faster** |
| `BTreeMap` | 213.38 µs | 4.80 M elem/s | 0.15× | 1.00× |

**Honest read:** HashMap wins point lookup decisively — that is its job.
ART beats BTreeMap by ~1.4× because the trie's path-compressed inner
nodes have better cache behavior than B-tree node search at this scale.
**ART is not a HashMap replacement.** If your workload is 100% point
lookups on randomly-distributed keys, use a hash table.

### Prefix scan (one file, ~100 results per prefix, 8 prefixes per iteration)

| Backend | Median time | Throughput | vs ART |
|---|---|---|---|
| **`blart::TreeMap` (ART)** | **1.12 µs** | **7.15 M scans/s** | **1.00×** |
| `BTreeMap` | 3.39 µs | 2.36 M scans/s | 3.03× slower |
| `HashMap` (full-scan + filter) | 6,395.80 µs | 1,250 scans/s | **5,720× slower** |

**This is the headline.** ART is the right primitive when prefix queries
are part of the workload:

- ART beats BTreeMap by **~3×** on this realistic AST-prefix workload.
  Both maintain lexicographic order; ART wins because its inner-node
  layout (Node4/16/48/256) is dramatically more cache-friendly than
  B-tree page traversal.
- HashMap loses by **three-and-a-half orders of magnitude** because it
  has no order — the only way to "prefix scan" a HashMap is to iterate
  every key and string-match. A coding agent that does even moderate
  prefix queries against a HashMap-backed symbol table is paying
  millisecond-scale latency per query while ART pays microsecond-scale.

### Insert (100k bulk insert, full reconstruction per iteration)

| Backend | Median time | Throughput |
|---|---|---|
| `HashMap` | 20.58 ms | 4.86 M elem/s |
| **`blart::TreeMap` (ART)** | **20.59 ms** | **4.86 M elem/s** |
| `BTreeMap` | 22.18 ms | 4.51 M elem/s |

ART's bulk insert is **competitive with HashMap** and slightly faster
than BTreeMap. Building a 100k-symbol index from scratch costs ~20 ms.
That means ingesting an entire mid-sized repo is a sub-second operation —
within the success-criterion budget of <500 ms for ≤50k LOC.

## Headline number

> **ART's prefix-scan latency stays roughly constant at ~570 ns from 10k
> to 1M keys, while HashMap full-scan grows linearly to 58.8 ms — a
> ~100,000× gap by 1M. ART beats `BTreeMap` by ~2.5–3× across the
> scaling sweep. Insert throughput is competitive with HashMap. Point
> lookup is faster than BTreeMap but slower than HashMap (as expected —
> ART trades point-lookup speed for ordered-prefix-scan capability).
> Per-backend isolated RSS at 1M keys: ART 187 MB, BTreeMap 224 MB,
> HashMap 223 MB — ART is ~16% smaller than both.**

See **Scaling**, **Real-data**, and **Memory (RSS)** sections below for
the supporting numbers and methodology.

## Why this matches the agent-memory use case

A coding agent's tool calls are mostly *structured-prefix queries*
("everything in `src/auth/`", "all functions in `parser.rs`",
"all `impl` blocks for type `Memory`"), with occasional point lookups
("symbol details for `parse_request`"). That mix is exactly where ART's
~3× prefix-scan advantage compounds across an agent's inner loop:

- 30 prefix queries per agent turn × 2 µs (ART) = 60 µs total memory wait.
- 30 prefix queries × 6 µs (BTreeMap) = 180 µs total — still cheap, but 3× more.
- 30 prefix queries × 6 ms (HashMap full-scan) = **180 ms** of wasted wall-clock — agent sits idle.

Multiplied across hundreds of turns and many agents, the choice of
backing index *measurably* changes how responsive an agentic system feels
and how much it costs to operate.

## Scaling

Phase 6 sweep — same prefix-scan workload, varying N. Each prefix
returns ~100 keys (one file's worth of symbols).

| N (keys) | ART prefix-scan | BTreeMap | HashMap full-scan | ART/BTree | ART vs HashMap |
|---|---|---|---|---|---|
| 10k | 547 ns | 1,487 ns | 221 µs | **2.7×** faster | **404×** faster |
| 100k | 569 ns | 1,519 ns | 3,297 µs | **2.7×** faster | **5,795×** faster |
| 1M | 573 ns | 1,448 ns | **58,800 µs** | **2.5×** faster | **102,605×** faster |
| 5M\* | (gated by `BLAZING_ART_BENCH_SCALE=full`) | | | | |

\* The 5M tier is gated to keep peak RSS below ~2.5 GB on default runs.
Set `BLAZING_ART_BENCH_SCALE=full` to include it.

**The shape of this table is the headline.** ART and BTreeMap both stay
roughly constant as N grows — they're trees, descent is O(log N) with
small per-node cost. **HashMap full-scan grows linearly** with N because
without lexicographic order, every prefix query becomes O(n) iteration.
At 1M keys, ART is **5 orders of magnitude faster than HashMap** for
prefix queries — a gap that grows with the dataset.

A coding agent's symbol index isn't 1k entries. A 100k-LOC project with
all declarations indexed is ~30k–80k symbols. Multiply by 5–20 ingested
repos and you're at 1M+ keys, where the ART/HashMap gap is real, not
theoretical.

## Real-data ingestion

Tree-sitter parse + ART symbol indexing, single-threaded, wall-clock.

| Corpus | Files | Symbols | Time | Symbols/sec | Status |
|---|---|---|---|---|---|
| `blazing-art-mcp/src/` | 8 | 136 | 0.018s | ~7,650/s | always runs |
| CPython `Lib/` | (set `CPYTHON_LIB_PATH`) | | | | gated |
| TypeScript `src/` | (set `TYPESCRIPT_SRC_PATH`) | | | | gated |

Ingest is single-threaded today. The constraint is parsing throughput,
not ART insert throughput (ART insert is competitive with HashMap, ~5M
ops/s at 100k). Parallelizing the file walk via `rayon` is a v0.2
candidate.

## Memory (RSS at 1M keys)

Three isolated runs, one backend per process — `BLAZING_ART_RSS_TARGET=
art|btreemap|hashmap` controls which one. Each measurement is
`getrusage(RUSAGE_SELF).ru_maxrss` after the build phase, minus the
baseline taken just after the synthetic key vector was generated.

| Backend | Peak RSS after build | Delta from baseline | vs ART |
|---|---|---|---|
| **ART** (`blart::TreeMap`) | 368 MB | **187 MB** | **1.00× (smallest)** |
| HashMap | 404 MB | 223 MB | +19% |
| BTreeMap | 405 MB | 225 MB | +20% |

**ART uses ~16–17% less RSS than either alternative** at 1M keys with
realistic prefix-shared keys. The advantage comes from path compression:
ART stores shared prefixes once per branch instead of redundantly per
key (BTreeMap, HashMap), and its node sizes (Node4/16/48/256) adapt to
actual fan-out instead of allocating fixed-capacity slots.

Earlier all-three-in-one-process numbers showed HashMap apparently
"smallest" — but `ru_maxrss` is a process-lifetime high watermark, so
the second and third backends inherit the watermark from the first,
masking their true cost. Always run isolated for clean numbers.

Footnote: `ru_maxrss` units differ by platform — bytes on macOS,
kilobytes on Linux. The bench harness `compile_error!`s on non-macOS to
prevent silent 1024× errors. Numbers above are macOS-only.

## What's NOT being benchmarked here (anti-claims)

- **Comparison to embeddings / vector search.** Different problem class
  (semantic vs exact). The two complement each other: structured
  queries → ART (deterministic, sub-microsecond); semantic queries →
  vectors (approximate, 50ms+). A future side-by-side wall-clock
  comparison would help anchor the "complementary, not competing"
  framing — flagged for v0.2.
- **Comparison to Tantivy / sled.** Different problem class (full-text
  ranked search; disk-backed transactional K/V). Mentioned in
  `CLAUDE.md` rationale.
- **Concurrent-write performance.** Single-threaded today
  (`Arc<RwLock<TreeMap>>`). Swap to `congee` (lock-free ART-OLC) for a
  future phase if multi-writer becomes a real workload.

## Reproducing

```bash
# All Criterion groups (point_lookup, prefix_scan, insert_bulk,
# scaling_sweep at 10k/100k/1M, real_data tier 1):
cargo bench --bench art_benchmarks

# Add the 5M scaling tier (requires ~2.5 GB peak RSS):
BLAZING_ART_BENCH_SCALE=full cargo bench --bench art_benchmarks

# Real-data tiers 2 and 3 (require local repos):
CPYTHON_LIB_PATH=$HOME/cpython/Lib cargo bench --bench art_benchmarks
TYPESCRIPT_SRC_PATH=$HOME/typescript/src cargo bench --bench art_benchmarks

# Per-backend isolated RSS measurement (run all three for the table above):
BLAZING_ART_RSS_TARGET=art      cargo bench --bench memory_rss -- --nocapture
BLAZING_ART_RSS_TARGET=btreemap cargo bench --bench memory_rss -- --nocapture
BLAZING_ART_RSS_TARGET=hashmap  cargo bench --bench memory_rss -- --nocapture
```

Hardware for all numbers above: Apple Silicon (Darwin 25.5.0, ARM64),
`rustc 1.88.0`, `profile = release`, `lto = true`, `codegen-units = 1`,
`mimalloc` global allocator.

Full Criterion HTML report: `target/criterion/report/index.html`.
