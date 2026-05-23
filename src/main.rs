//! Binary entry point. Parses CLI flags, loads optional preload data into
//! `Memory`, then runs either the stdio or HTTP transport.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use blazing_art_mcp::memory::Memory;
use blazing_art_mcp::transport::{http, stdio};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(name = "blazing_art_mcp", about = "ART-backed MCP memory server")]
struct Cli {
    /// Optional JSON file of Entity records to preload.
    #[arg(long)]
    entities: Option<PathBuf>,

    /// Optional JSON file of Event records to preload.
    #[arg(long)]
    events: Option<PathBuf>,

    /// Cap for findEvents prefix scan.
    #[arg(long, default_value_t = 100)]
    event_limit: usize,

    /// If set, run in HTTP+SSE mode bound to this address (e.g. `127.0.0.1:4242`).
    /// If unset, run in stdio mode.
    #[arg(long)]
    http: Option<SocketAddr>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let memory = Arc::new(Memory::new(cli.event_limit));
    if let Some(p) = cli.entities.as_ref() {
        memory.load_entities(p).context("loading entities")?;
    }
    if let Some(p) = cli.events.as_ref() {
        memory.load_events(p).context("loading events")?;
    }

    if let Some(addr) = cli.http {
        // Spec security: bind to localhost only. Reject anything broader.
        if !addr.ip().is_loopback() {
            anyhow::bail!(
                "--http must bind to a loopback address (got {}); the MCP spec requires \
                 localhost-only binding for security. Use 127.0.0.1 or [::1].",
                addr.ip()
            );
        }
        http::run(memory, addr).await
    } else {
        stdio::run(memory).await
    }
}
