# Enhanced Production Dockerfile with security and performance optimizations
# ---------- build stage ----------
FROM rust:1.76-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    clang \
    lld \
    git \
    pkgconfig \
    openssl-dev

# Create app user for security
RUN addgroup -g 1001 -S appuser && \
    adduser -S appuser -u 1001

WORKDIR /app

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./

# Add target and pre-fetch dependencies
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo fetch --target x86_64-unknown-linux-musl

# Copy source code
COPY src ./src

# Build with optimizations - static binary for scratch image
ENV RUSTFLAGS="-C target-feature=+crt-static -C link-arg=-s"
RUN cargo build \
    --release \
    --target x86_64-unknown-linux-musl \
    --offline

# Generate SBOM for security (optional - requires cargo-cyclonedx)
# RUN cargo install cargo-cyclonedx --version 0.5.3 && \
#     cargo cyclonedx --format json --output /app/sbom.json

# ---------- security scanning stage ----------
FROM aquasec/trivy:latest AS security-scan
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/blazing_art_mcp /tmp/
RUN trivy fs --security-checks vuln --format sarif --output /tmp/trivy-report.sarif /tmp/

# ---------- runtime stage ----------
FROM gcr.io/distroless/static:nonroot

# Copy binary with correct permissions
COPY --from=builder --chown=nonroot:nonroot \
    /app/target/x86_64-unknown-linux-musl/release/blazing_art_mcp \
    /blazing_art_mcp

# Security labels
LABEL \
    org.opencontainers.image.title="MCP Memory Server" \
    org.opencontainers.image.description="High-performance ART-backed MCP server" \
    org.opencontainers.image.vendor="Your Organization" \
    org.opencontainers.image.licenses="MIT" \
    org.opencontainers.image.source="https://github.com/JohnJBoren/mcp-memory-server" \
    security.scan.enabled="true"

# Use non-root user (distroless provides nonroot user)
USER nonroot:nonroot

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/blazing_art_mcp", "--health-check"]

# Default to STDIO transport
ENTRYPOINT ["/blazing_art_mcp"]

# ---------- Development variant ----------
FROM builder AS development
RUN cargo install cargo-watch
COPY . .
CMD ["cargo", "watch", "-x", "run"]

# ---------- Build instructions ----------
# Build production image:
# docker build --target runtime -t mcp-memory:latest .
#
# Build with security scan:
# docker build --target security-scan -t mcp-memory:scan .
# docker run --rm mcp-memory:scan cat /tmp/trivy-report.sarif
#
# Development mode:
# docker build --target development -t mcp-memory:dev .
# docker run -v $(pwd):/app mcp-memory:dev
