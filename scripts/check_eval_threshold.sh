#!/usr/bin/env bash
# v0.2 Task 8: CI gate for the gold-set eval.
#
# Reads the most recent eval/results/<sha>-<ts>.json and compares each metric
# against eval/threshold.json. Exits 0 if all gates pass, 1 if any gate fails.
#
# Designed to be called from .github/workflows/ci.yml after `eval_goldset` runs.
#
# Requirements: bash, jq.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS_DIR="$ROOT/eval/results"
THRESHOLD="$ROOT/eval/threshold.json"

if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required but not on PATH. Install via 'brew install jq' or 'apt-get install jq'." >&2
    exit 2
fi

if [ ! -d "$RESULTS_DIR" ]; then
    echo "ERROR: no results directory at $RESULTS_DIR. Did you run 'cargo run --release --bin eval_goldset' first?" >&2
    exit 2
fi

# Pick the newest results JSON.
LATEST="$(ls -t "$RESULTS_DIR"/*.json 2>/dev/null | head -1 || true)"
if [ -z "$LATEST" ]; then
    echo "ERROR: no eval results found under $RESULTS_DIR" >&2
    exit 2
fi
echo "[gate] checking $LATEST against $THRESHOLD"

# Helper: compare $1 (got) >= $2 (floor); echo PASS/FAIL with label.
check_ge() {
    local label="$1"; local got="$2"; local floor="$3"
    awk -v g="$got" -v f="$floor" -v lbl="$label" 'BEGIN {
        if (g + 0 >= f + 0) { printf "  PASS  %-32s %.3f >= %.3f\n", lbl, g, f; exit 0 }
        else                { printf "  FAIL  %-32s %.3f <  %.3f\n", lbl, g, f; exit 1 }
    }'
}

failed=0

# Overall recall + MRR floors.
got_at_1=$(jq -r '.overall.recall_at_1' "$LATEST")
got_at_5=$(jq -r '.overall.recall_at_5' "$LATEST")
got_at_20=$(jq -r '.overall.recall_at_20' "$LATEST")
got_mrr=$(jq -r '.overall.mrr' "$LATEST")

floor_at_1=$(jq -r '.recall.at_1' "$THRESHOLD")
floor_at_5=$(jq -r '.recall.at_5' "$THRESHOLD")
floor_at_20=$(jq -r '.recall.at_20' "$THRESHOLD")
floor_mrr=$(jq -r '.mrr.floor' "$THRESHOLD")

check_ge "overall.recall_at_1"  "$got_at_1"  "$floor_at_1"  || failed=1
check_ge "overall.recall_at_5"  "$got_at_5"  "$floor_at_5"  || failed=1
check_ge "overall.recall_at_20" "$got_at_20" "$floor_at_20" || failed=1
check_ge "overall.mrr"          "$got_mrr"   "$floor_mrr"   || failed=1

# Per-category floors. Iterate keys in threshold.json.categories.
for cat in $(jq -r '.categories | keys[]' "$THRESHOLD"); do
    floor_at_5_cat=$(jq -r ".categories[\"$cat\"].recall_at_5 // empty" "$THRESHOLD")
    if [ -n "$floor_at_5_cat" ]; then
        got_cat=$(jq -r ".by_category[\"$cat\"].recall_at_5 // 0" "$LATEST")
        check_ge "categories.$cat.recall_at_5" "$got_cat" "$floor_at_5_cat" || failed=1
    fi
    floor_empty=$(jq -r ".categories[\"$cat\"].empty_correctness // empty" "$THRESHOLD")
    if [ -n "$floor_empty" ]; then
        got_empty=$(jq -r ".by_category[\"$cat\"].empty_correctness // 0" "$LATEST")
        check_ge "categories.$cat.empty_correctness" "$got_empty" "$floor_empty" || failed=1
    fi
done

if [ "$failed" -ne 0 ]; then
    echo ""
    echo "[gate] FAILED — at least one metric below floor. See $LATEST for full report."
    exit 1
fi
echo ""
echo "[gate] OK — all eval thresholds satisfied."
