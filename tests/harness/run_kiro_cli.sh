#!/usr/bin/env bash
# v0.2 Task 10: Live integration test driving kiro-cli (`kiro-cli chat
# --no-interactive --trust-all-tools`) against the blazing-art MCP server.
#
# Mirror of run_claude_code.sh: same prompts (tests/harness/scripts/) and
# same assertion table (tests/harness/asserts.txt). Difference:
#   * kiro-cli loads MCP servers via `kiro-cli mcp add` (workspace scope),
#     not a config file.
#   * kiro-cli runs the server as a stdio subprocess, not an HTTP client —
#     so this script does NOT pre-spawn the binary; kiro-cli does.
#
# Skips gracefully if kiro-cli isn't on PATH.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPTS_DIR="$ROOT/tests/harness/scripts"
ASSERTS="$ROOT/tests/harness/asserts.txt"
TRANSCRIPT_DIR="$ROOT/tests/harness/transcripts"
mkdir -p "$TRANSCRIPT_DIR"

# --- Skip checks -----------------------------------------------------------

if ! command -v kiro-cli >/dev/null 2>&1; then
    echo "[harness/kiro-cli] SKIP: kiro-cli not on PATH"
    exit 0
fi

# --- Build + register MCP server -------------------------------------------

echo "[harness/kiro-cli] building release binary…"
cargo build --release --bin blazing_art_mcp >&2

SERVER_BIN="$ROOT/target/release/blazing_art_mcp"
SERVER_NAME="blazing-art-harness"

# Always remove any leftover registration first (idempotent).
kiro-cli mcp remove --name "$SERVER_NAME" --scope workspace 2>/dev/null || true

cleanup() {
    kiro-cli mcp remove --name "$SERVER_NAME" --scope workspace 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "[harness/kiro-cli] registering blazing-art as workspace MCP server…"
kiro-cli mcp add \
    --name "$SERVER_NAME" \
    --scope workspace \
    --force \
    --command "$SERVER_BIN" \
    --args "--entities,$ROOT/data/entities.json,--events,$ROOT/data/events.json"

# Optional: bump non-interactive timeout for slower MCP server startups.
kiro-cli settings mcp.noInteractiveTimeout 60000 >/dev/null 2>&1 || true

# --- Per-prompt assertion lookup (bash-3.2-portable; macOS default) --------
get_re() {
    local id="$1"; local kind="$2"  # kind = "T" or "C-"
    awk -v want_id="$id" -v want_kind="$kind:" '
        BEGIN { in_block = 0 }
        /^[[:space:]]*$/        { in_block = 0; next }
        /^[[:space:]]*#/        { next }
        $0 == want_id           { in_block = 1; next }
        in_block && index($0, want_kind) == 1 {
            sub(want_kind, "", $0); print; exit
        }
    ' "$ASSERTS"
}

# --- Drive kiro-cli per prompt ---------------------------------------------

pass_t=0; fail_t=0
pass_c=0; fail_c=0

for script_path in "$SCRIPTS_DIR"/*.txt; do
    name="$(basename "$script_path" .txt)"
    id="${name%%_*}"
    prompt="$(cat "$script_path")"

    treatment_log="$TRANSCRIPT_DIR/${name}.kiro.treatment.txt"
    control_log="$TRANSCRIPT_DIR/${name}.kiro.control.txt"

    echo "[harness/kiro-cli] $name"

    # Treatment: kiro-cli chat with the registered MCP server in workspace scope.
    kiro-cli chat --no-interactive --trust-all-tools "$prompt" \
        > "$treatment_log" 2>&1 || true

    # Control: temporarily disable our server, re-run, re-enable.
    kiro-cli mcp remove --name "$SERVER_NAME" --scope workspace 2>/dev/null || true
    kiro-cli chat --no-interactive --trust-all-tools "$prompt" \
        > "$control_log" 2>&1 || true
    kiro-cli mcp add \
        --name "$SERVER_NAME" --scope workspace --force \
        --command "$SERVER_BIN" \
        --args "--entities,$ROOT/data/entities.json,--events,$ROOT/data/events.json"

    t_re="$(get_re "$id" T)"
    c_re="$(get_re "$id" C-)"

    if [ -n "$t_re" ]; then
        if grep -Eqi "$t_re" "$treatment_log"; then
            echo "    T PASS  /$t_re/"
            pass_t=$((pass_t + 1))
        else
            echo "    T FAIL  /$t_re/  (see $treatment_log)"
            fail_t=$((fail_t + 1))
        fi
    fi
    if [ -n "$c_re" ]; then
        if grep -Eqi "$c_re" "$control_log"; then
            echo "    C FAIL  control answered without MCP (regex /$c_re/ matched)"
            fail_c=$((fail_c + 1))
        else
            echo "    C PASS  control did NOT match (correct)"
            pass_c=$((pass_c + 1))
        fi
    fi
done

echo ""
echo "[harness/kiro-cli] SUMMARY"
echo "  Treatment regex hits (must match): $pass_t passed, $fail_t failed"
echo "  Control   regex hits (must MISS):  $pass_c passed, $fail_c failed"
echo ""

if [ "$fail_t" -gt 0 ] || [ "$fail_c" -gt 0 ]; then
    echo "[harness/kiro-cli] OVERALL: FAIL"
    exit 1
fi
echo "[harness/kiro-cli] OVERALL: OK"
