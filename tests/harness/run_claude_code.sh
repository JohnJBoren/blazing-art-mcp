#!/usr/bin/env bash
# v0.2 Task 9: Live integration test driving Claude Code (`claude -p` headless mode)
# against the blazing-art MCP server.
#
# For each prompt under tests/harness/scripts/, runs Claude Code twice:
#   * Treatment: with the blazing-art MCP server enabled.
#   * Control:   with no MCP servers configured.
# Asserts the treatment transcript matches a per-prompt regex AND the control
# transcript does NOT match a separate regex (i.e. that the question is
# answerable only with our index in the loop).
#
# Skips gracefully if `claude` isn't on PATH or `ANTHROPIC_API_KEY` isn't set.
# Always tears down the spawned MCP server via a `trap` so re-runs are clean.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPTS_DIR="$ROOT/tests/harness/scripts"
ASSERTS="$ROOT/tests/harness/asserts.txt"
TRANSCRIPT_DIR="$ROOT/tests/harness/transcripts"
mkdir -p "$TRANSCRIPT_DIR"

# --- Skip checks -----------------------------------------------------------

if ! command -v claude >/dev/null 2>&1; then
    echo "[harness/claude] SKIP: \`claude\` CLI not on PATH"
    exit 0
fi
if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
    echo "[harness/claude] SKIP: ANTHROPIC_API_KEY not set in env"
    exit 0
fi

# --- Build + spawn server --------------------------------------------------

echo "[harness/claude] building release binary…"
cargo build --release --bin blazing_art_mcp >&2

PORT="${BLAZING_ART_HARNESS_PORT:-4243}"
SERVER_PID=""
TMPCONFIG="$(mktemp -d)/claude_mcp_config.json"

cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$(dirname "$TMPCONFIG")"
}
trap cleanup EXIT INT TERM

echo "[harness/claude] spawning MCP server on 127.0.0.1:$PORT"
"$ROOT/target/release/blazing_art_mcp" --http "127.0.0.1:$PORT" >"$TRANSCRIPT_DIR/server.log" 2>&1 &
SERVER_PID="$!"

# Wait for /health.
for _ in $(seq 1 50); do
    if curl -sfL "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if ! curl -sfL "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo "[harness/claude] FAIL: server never came up healthy" >&2
    exit 1
fi
echo "[harness/claude]   server PID=$SERVER_PID, /health OK"

# --- Per-prompt assertion lookup (bash-3.2-portable; macOS default) --------
# Lookup helper: emits the regex for `<id>` of type `T:` or `C-:`, or empty.
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

# --- Write Claude Code MCP config (treatment) ------------------------------

cat > "$TMPCONFIG" <<EOF
{
  "mcpServers": {
    "blazing-art": {
      "transport": "http",
      "url": "http://127.0.0.1:$PORT/mcp"
    }
  }
}
EOF

# --- Drive Claude Code per prompt ------------------------------------------

pass_t=0; fail_t=0
pass_c=0; fail_c=0

for script_path in "$SCRIPTS_DIR"/*.txt; do
    name="$(basename "$script_path" .txt)"
    id="${name%%_*}"
    prompt="$(cat "$script_path")"

    treatment_log="$TRANSCRIPT_DIR/${name}.treatment.txt"
    control_log="$TRANSCRIPT_DIR/${name}.control.txt"

    echo "[harness/claude] $name"

    # Treatment: with the blazing-art MCP server.
    claude -p "$prompt" --mcp-config "$TMPCONFIG" --allowedTools "mcp__blazing-art__ingestRepo,mcp__blazing-art__findSymbols,mcp__blazing-art__findReferences" \
        > "$treatment_log" 2>&1 || true

    # Control: no MCP server.
    claude -p "$prompt" \
        > "$control_log" 2>&1 || true

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
        # Control should NOT match (i.e. control can't answer correctly without MCP).
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
echo "[harness/claude] SUMMARY"
echo "  Treatment regex hits (must match): $pass_t passed, $fail_t failed"
echo "  Control   regex hits (must MISS):  $pass_c passed, $fail_c failed"
echo ""

if [ "$fail_t" -gt 0 ] || [ "$fail_c" -gt 0 ]; then
    echo "[harness/claude] OVERALL: FAIL"
    exit 1
fi
echo "[harness/claude] OVERALL: OK"
