//! Stdio transport: read newline-delimited JSON-RPC frames from stdin, write
//! responses to stdout. Stderr is reserved for diagnostics. Per the MCP spec,
//! stdout MUST contain only valid JSON-RPC messages.

use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::memory::Memory;
use crate::protocol::{dispatch, JsonRpcError, JsonRpcRequest, JsonRpcResponse};

pub async fn run(memory: Arc<Memory>) -> Result<()> {
    eprintln!("Blazing-ART-MCP server started (STDIO mode)");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut stdout = stdout;

    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                eprintln!("EOF received, shutting down gracefully");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                eprintln!("Received request: {trimmed}");

                match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                    Ok(request) => {
                        if let Some(response) = dispatch(&memory, request) {
                            let response_str = serde_json::to_string(&response)?;
                            eprintln!("Sending response: {response_str}");

                            if let Err(e) = stdout.write_all(response_str.as_bytes()).await {
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    eprintln!("Client closed connection");
                                    break;
                                }
                                return Err(e.into());
                            }
                            if let Err(e) = stdout.write_all(b"\n").await {
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    break;
                                }
                                return Err(e.into());
                            }
                            if let Err(e) = stdout.flush().await {
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    break;
                                }
                                return Err(e.into());
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to parse request: {e}");
                        let error_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: serde_json::Value::Null,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32700,
                                message: format!("Parse error: {e}"),
                            }),
                        };
                        let response_str = serde_json::to_string(&error_response)?;
                        stdout.write_all(response_str.as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading input: {e}");
                break;
            }
        }
    }

    eprintln!("MCP server shutting down");
    Ok(())
}
