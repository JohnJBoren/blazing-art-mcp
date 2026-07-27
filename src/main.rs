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

    /// v0.3 Task 5: persistence. If set, the server loads from this file on
    /// startup (when present) and writes a snapshot back to it on Ctrl+C /
    /// SIGTERM. The file format is bincode of the full Memory + 8-byte magic
    /// header `BARTS001`. Atomic on disk via tmp + rename.
    #[arg(long)]
    snapshot_path: Option<PathBuf>,

    /// v0.3 Task 6: live file-watcher. Format `<repo_id>=<path>`, repeatable.
    /// On startup the path is ingested with the given repo_id; thereafter the
    /// server watches the directory and re-ingests individual files on change
    /// (debounced ~300ms). Example:
    ///   --watch self=$PWD/src --watch fixture=$PWD/tests/fixtures/sample-repo
    #[arg(long, value_name = "REPO_ID=PATH")]
    watch: Vec<String>,
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

    // v0.3 Task 5: load snapshot if --snapshot-path given and the file exists.
    if let Some(p) = cli.snapshot_path.as_ref() {
        if p.exists() {
            memory.load_snapshot(p).context("loading snapshot")?;
        } else {
            eprintln!(
                "[snapshot] --snapshot-path {} does not exist yet; will write on shutdown",
                p.display()
            );
        }

        // Register a Ctrl+C / SIGTERM handler that snapshots before exit.
        // The closure captures Arc<Memory> + path; the ctrlc crate spawns its
        // own thread so this is safe to register from async main.
        let mem_for_signal = Arc::clone(&memory);
        let path_for_signal = p.clone();
        ctrlc::set_handler(move || {
            eprintln!("[snapshot] received signal — writing final snapshot");
            if let Err(e) = mem_for_signal.snapshot(&path_for_signal) {
                eprintln!("[snapshot] FAILED on shutdown: {e}");
            }
            std::process::exit(0);
        })
        .context("installing ctrlc handler")?;
    }

    // v0.3 Task 6: file watchers. One per --watch <repo_id>=<path>.
    if !cli.watch.is_empty() {
        for spec in &cli.watch {
            let (repo_id, path_str) = spec
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--watch must be REPO_ID=PATH; got {spec}"))?;
            let path = PathBuf::from(path_str).canonicalize().with_context(|| {
                format!("--watch path does not exist or cannot be canonicalized: {path_str}")
            })?;
            let repo_id = repo_id.to_string();

            // Initial cold ingest.
            eprintln!("[watch] initial ingest of {repo_id} from {}", path.display());
            let stats = blazing_art_mcp::ingest::ingest_repo(&memory, &repo_id, &path);
            eprintln!(
                "[watch]   files={} symbols={} errors={}",
                stats.files_parsed, stats.symbols_indexed, stats.errors.len()
            );

            // Spawn the watcher thread.
            let mem = Arc::clone(&memory);
            let watch_root = path.clone();
            std::thread::spawn(move || {
                use notify_debouncer_mini::new_debouncer;
                use notify_debouncer_mini::notify::RecursiveMode;
                use std::time::Duration;

                let (tx, rx) = std::sync::mpsc::channel();
                let mut debouncer = match new_debouncer(Duration::from_millis(300), tx) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[watch/{repo_id}] failed to create debouncer: {e}");
                        return;
                    }
                };
                if let Err(e) = debouncer.watcher().watch(&watch_root, RecursiveMode::Recursive) {
                    eprintln!("[watch/{repo_id}] failed to watch {}: {e}", watch_root.display());
                    return;
                }
                eprintln!("[watch/{repo_id}] live; debounce=300ms; recursive on {}", watch_root.display());

                for batch in rx {
                    let events = match batch {
                        Ok(events) => events,
                        Err(errs) => {
                            for e in errs {
                                eprintln!("[watch/{repo_id}] error: {e:?}");
                            }
                            continue;
                        }
                    };
                    for ev in events {
                        let p = ev.path.clone();
                        match blazing_art_mcp::ingest::reingest_file(&mem, &repo_id, &watch_root, &p) {
                            Ok((removed, inserted)) => {
                                if removed > 0 || inserted > 0 {
                                    eprintln!(
                                        "[watch/{repo_id}] {} -{} +{}",
                                        p.display(),
                                        removed,
                                        inserted
                                    );
                                }
                            }
                            Err(e) => eprintln!("[watch/{repo_id}] {} ERR: {e}", p.display()),
                        }
                    }
                }
            });
        }
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
