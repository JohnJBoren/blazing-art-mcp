# Retrieval-quality eval

This directory holds the v0.2 Task 7/8 gold-set + eval results.

```
eval/
├── README.md            # this file
├── goldset/
│   ├── *.jsonl          # generated per-repo by `build_goldset`
│   └── handcurated.jsonl  # 30 hand-authored hard cases (Task 7)
├── threshold.toml       # CI gate thresholds (Task 8)
└── results/
    └── <git-sha>-<ts>.{json,md}   # per-run scoring output
```

## Schema (one JSON object per line)

```json
{
  "query":         "natural-language question or commit subject",
  "repo":          "repo_id used during ingest",
  "path":          "rel/path/to/file.ext",
  "kind":          "function | class | method | type | interface | module | macro | constant",
  "name":          "exact symbol name",
  "role":          "definition | reference",
  "source_commit": "abbrev sha (auto-generated records only)",

  // Only on hand-curated records:
  "category":      "ambiguous | cross_language | rename | long_tail | negative",
  "must_be_empty": false  // negative cases set this to true and omit path/kind/name
}
```

## Generating an auto-derived set

```bash
cargo run --release --bin build_goldset -- \
    --repo /path/to/some/repo \
    --out  eval/goldset/some-repo.jsonl \
    --max-records 200 \
    --lookback-commits 5000
```

The generator skips merges, short subjects (<6 words), and project-meta
boilerplate (bump/fmt/chore/wip/...). For each remaining commit subject it
extracts identifier-like tokens and emits a record whenever exactly one
declaration in the index uniquely matches.

## Running the eval

See Task 8 deliverables (`src/bin/eval_goldset.rs` + `scripts/check_eval_threshold.sh`).
