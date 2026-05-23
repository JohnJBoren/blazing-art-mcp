//! HTTP transport (MCP Streamable HTTP, spec version 2025-06-18).
//!
//! Single MCP endpoint at `/mcp` handling POST and GET. Plus `/health` and
//! a static demo `index.html` at `/` (loaded from disk at request time so
//! the same binary works whether or not the html file exists).
//!
//! Per spec section "Streamable HTTP":
//!   - Server MUST validate the Origin header to defeat DNS rebinding.
//!   - Server SHOULD bind only to localhost.
//!   - POST request body is one JSON-RPC message.
//!   - For requests, server MAY return application/json or text/event-stream.
//!     We return application/json since our four tools are synchronous.
//!   - For notifications/responses, server MUST return 202 Accepted no body.
//!   - GET MAY open an SSE stream for server-initiated messages. We don't
//!     emit any, so we return 405 Method Not Allowed (also spec-compliant).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

use crate::memory::Memory;
use crate::protocol::{dispatch, is_notification, JsonRpcRequest};

/// Hosts the request-handler shares with axum extractors.
#[derive(Clone)]
struct AppState {
    memory: Arc<Memory>,
}

/// Validate the Origin header (when present) against an allow-list.
/// We accept localhost and 127.0.0.1 on any port plus null/missing (curl, MCP clients).
/// Any other Origin is rejected with 403.
fn origin_is_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true; // Missing Origin is acceptable for non-browser clients.
    };
    let Ok(origin_str) = origin.to_str() else {
        return false;
    };
    // Normalize: strip scheme; check the host.
    // Accepted patterns: http://localhost(:port), http://127.0.0.1(:port), null.
    let lower = origin_str.to_ascii_lowercase();
    if lower == "null" {
        return true;
    }
    let host_part = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .unwrap_or(&lower);
    let host = host_part.split('/').next().unwrap_or("");
    let host_only = host.split(':').next().unwrap_or("");
    matches!(host_only, "localhost" | "127.0.0.1" | "[::1]")
}

async fn handle_health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// Serve the demo HTML page. If the file is missing (e.g. binary deployed
/// without the static/ directory) we return a small inline placeholder.
async fn handle_index() -> impl IntoResponse {
    let path = std::path::Path::new("static/index.html");
    if let Ok(body) = tokio::fs::read_to_string(path).await {
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
    } else {
        let inline = "<!doctype html><meta charset=utf-8><title>blazing-art-mcp</title>\
            <h1>blazing-art-mcp</h1>\
            <p>HTTP transport is up. The web UI lives at <code>static/index.html</code> \
            but was not found on disk. POST JSON-RPC to <code>/mcp</code>, or check \
            <code>/health</code>.</p>";
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], inline.to_string()).into_response()
    }
}

/// POST /mcp — accept one JSON-RPC message, dispatch, return response.
async fn handle_mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !origin_is_allowed(&headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "error": {"code": -32600, "message": "Origin not allowed"},
                "id": null
            })),
        )
            .into_response();
    }

    let request: JsonRpcRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {"code": -32700, "message": format!("Parse error: {e}")},
                    "id": null
                })),
            )
                .into_response();
        }
    };

    // Per spec: notifications and responses get 202 Accepted with no body.
    if is_notification(&request) {
        // Still let dispatch handle any side-effects (none for our methods today).
        let _ = dispatch(&state.memory, request);
        return StatusCode::ACCEPTED.into_response();
    }

    match dispatch(&state.memory, request) {
        Some(response) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::to_value(response).unwrap_or(Value::Null)),
        )
            .into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// GET /mcp — per spec, MAY open an SSE stream for server-initiated messages.
/// We don't emit any, so return 405 (spec-compliant).
async fn handle_mcp_get(headers: HeaderMap) -> Response {
    if !origin_is_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        "GET on /mcp not supported (server emits no proactive messages)",
    )
        .into_response()
}

pub async fn run(memory: Arc<Memory>, addr: SocketAddr) -> Result<()> {
    let state = AppState { memory };

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/health", get(handle_health))
        .route("/mcp", post(handle_mcp_post).get(handle_mcp_get))
        .with_state(state);

    eprintln!("Blazing-ART-MCP server started (HTTP mode) on http://{addr}");
    eprintln!("  POST /mcp     — JSON-RPC endpoint");
    eprintln!("  GET  /health  — liveness probe");
    eprintln!("  GET  /        — demo UI (if static/index.html present)");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    axum::serve(listener, app).await?;
    Ok(())
}
