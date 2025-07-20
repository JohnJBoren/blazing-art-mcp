#!/bin/bash
# MCP wrapper script that redirects stderr to avoid protocol interference

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"

# Run the MCP server with stderr redirected to a log file
exec "$SCRIPT_DIR/target/release/blazing_art_mcp" "$@" 2>>"$SCRIPT_DIR/mcp-server.log"