#!/bin/bash

# Blazing-ART-MCP Server Startup Script
# This script provides convenient ways to run the MCP server

# Set script directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
cd "$SCRIPT_DIR"

# Default settings
ENTITIES_FILE="${ENTITIES_FILE:-$SCRIPT_DIR/data/entities.json}"
EVENTS_FILE="${EVENTS_FILE:-$SCRIPT_DIR/data/events.json}"
EVENT_LIMIT="${EVENT_LIMIT:-100}"
WS_PORT="${WS_PORT:-4000}"
HEALTH_PORT="${HEALTH_PORT:-3000}"

# Check if binary exists
if [ ! -f "$SCRIPT_DIR/target/release/blazing_art_mcp" ]; then
    echo "Error: blazing_art_mcp binary not found!"
    echo "Please run: cargo build --release"
    exit 1
fi

# Parse command line arguments
case "$1" in
    "stdio")
        echo "Starting Blazing-ART-MCP in STDIO mode..."
        exec "$SCRIPT_DIR/target/release/blazing_art_mcp" \
            --entities "$ENTITIES_FILE" \
            --events "$EVENTS_FILE" \
            --event-limit "$EVENT_LIMIT"
        ;;
    
    "websocket"|"ws")
        echo "Starting Blazing-ART-MCP in WebSocket mode on port $WS_PORT..."
        exec "$SCRIPT_DIR/target/release/blazing_art_mcp" \
            --ws "0.0.0.0:$WS_PORT" \
            --health-port "$HEALTH_PORT" \
            --entities "$ENTITIES_FILE" \
            --events "$EVENTS_FILE" \
            --event-limit "$EVENT_LIMIT"
        ;;
    
    "dev")
        echo "Starting Blazing-ART-MCP in development mode with telemetry..."
        RUST_LOG=debug exec "$SCRIPT_DIR/target/release/blazing_art_mcp" \
            --ws "0.0.0.0:$WS_PORT" \
            --health-port "$HEALTH_PORT" \
            --entities "$ENTITIES_FILE" \
            --events "$EVENTS_FILE" \
            --event-limit "$EVENT_LIMIT" \
            --telemetry
        ;;
    
    "health")
        echo "Running health check..."
        exec "$SCRIPT_DIR/target/release/blazing_art_mcp" --health-check
        ;;
    
    *)
        echo "Blazing-ART-MCP Server Launcher"
        echo ""
        echo "Usage: $0 [stdio|websocket|ws|dev|health]"
        echo ""
        echo "Modes:"
        echo "  stdio      - Run in STDIO mode (default for MCP)"
        echo "  websocket  - Run with WebSocket transport on port $WS_PORT"
        echo "  ws         - Alias for websocket"
        echo "  dev        - Run in development mode with debug logging"
        echo "  health     - Run health check"
        echo ""
        echo "Environment variables:"
        echo "  ENTITIES_FILE  - Path to entities JSON file (default: $ENTITIES_FILE)"
        echo "  EVENTS_FILE    - Path to events JSON file (default: $EVENTS_FILE)"
        echo "  EVENT_LIMIT    - Maximum events to return (default: $EVENT_LIMIT)"
        echo "  WS_PORT        - WebSocket port (default: $WS_PORT)"
        echo "  HEALTH_PORT    - Health check port (default: $HEALTH_PORT)"
        echo ""
        echo "Examples:"
        echo "  $0 stdio                    # Run for Claude Desktop"
        echo "  $0 websocket                # Run WebSocket server"
        echo "  WS_PORT=5000 $0 ws         # Run on custom port"
        echo "  RUST_LOG=trace $0 dev      # Run with trace logging"
        exit 1
        ;;
esac