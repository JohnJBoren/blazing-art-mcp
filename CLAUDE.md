# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Essential Commands

### Building and Running
```bash
# Standard build
cargo build
cargo build --release

# Run with sample data
cargo run -- --entities examples/entities.json --events examples/events.json

# Run with WebSocket mode
cargo run -- --ws 0.0.0.0:4000 --health-port 3000 --entities examples/entities.json --events examples/events.json

# Hot reload development
cargo watch -x 'run -- --entities examples/entities.json --events examples/events.json'
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --test-threads=1 --nocapture

# Run benchmarks
cargo bench
```

### Code Quality
```bash
# Linting (must pass without warnings)
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt --all

# Security audit
cargo audit
```

### Docker Development
```bash
# Build production image
docker build -t mcp-memory:latest .

# Build development image with hot reload
docker build --target development -t mcp-memory:dev .

# Run security scan
docker build --target security-scan -t mcp-memory:scan .
docker run --rm mcp-memory:scan cat /tmp/trivy-report.sarif
```

## Architecture Overview

This is a high-performance MCP (Model Context Protocol) server built in Rust that provides microsecond-latency memory access for LLMs using Adaptive Radix Trees (ART).

### Core Components

1. **Memory Backend**: Uses `art-tree` crate for O(k) operations with cache-friendly memory layout
2. **MCP Protocol**: Implements Model Context Protocol via `rmcp` for LLM integration
3. **Transport Layers**: 
   - STDIO mode for sidecar deployment
   - WebSocket mode for network access
4. **Performance Optimizations**:
   - Custom memory allocator (mimalloc/jemalloc)
   - Zero-copy serialization with rkyv
   - Lock-free data structures where possible
   - Cache-aligned memory structures

### Key Files

- `src/main.rs`: Single-file implementation containing:
  - CLI argument parsing
  - ART-backed memory stores for entities and events
  - MCP tool implementations (lookupEntity, findEvents)
  - Health check endpoints via Axum
  - Graceful shutdown handling
  - OpenTelemetry instrumentation

### Data Structures

The server manages two main data types:

1. **Entity**: Person/organization with name, summary, born date, and tags
2. **Event**: Timestamped event with ID, description, and category

Both use rkyv for zero-copy serialization and are indexed in separate ART instances.

### Production Features

- **Container-first**: Multi-stage Dockerfile with distroless runtime
- **Kubernetes-ready**: Full deployment manifests with RBAC, HPA, PDB
- **Observable**: Prometheus metrics, OpenTelemetry tracing, structured logging
- **Secure**: Non-root containers, read-only filesystems, vulnerability scanning

### Development Workflow

1. Make changes to `src/main.rs`
2. Run `cargo clippy -- -D warnings` to catch issues
3. Run `cargo fmt --all` to format code
4. Run `cargo test` to verify functionality
5. Test with sample data: `cargo run -- --entities examples/entities.json`
6. Build optimized: `cargo build --release`

### Performance Considerations

- The custom allocator is critical for performance (2-6x improvement)
- JSON serialization dominates latency, not ART operations
- Prefix scans are limited by `--event-limit` flag (default 64)
- Memory usage is ~12-85 MB for 100k-1M keys

### Deployment Options

1. **Local/Sidecar**: Use STDIO mode for direct integration
2. **Network Service**: Use WebSocket mode with health checks
3. **Kubernetes**: Apply `k8s/deployment.yaml` for production
4. **Docker Compose**: Use for full development environment with monitoring