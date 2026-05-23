//! JSON-RPC 2.0 message types and the tool-dispatch core.
//!
//! `dispatch()` is transport-agnostic: it takes a parsed `JsonRpcRequest` and a
//! `Memory`, returns `Option<JsonRpcResponse>` (None for notifications). Both
//! the stdio and HTTP transports call into it.

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ingest;
use crate::memory::{Entity, Event, Memory};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "blazing-art-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Serialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// Build the static `tools/list` payload. Pulled out so HTTP and stdio paths
/// produce identical schemas.
fn tools_list_payload() -> Value {
    serde_json::json!({
        "tools": [
            {
                "name": "lookupEntity",
                "description": "Retrieve stored information about an entity by exact name.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "The exact name of the entity to look up"}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "addEntity",
                "description": "Add or update an entity in the memory store.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "The name of the entity"},
                        "summary": {"type": "string", "description": "A summary of the entity"},
                        "born": {"type": "string", "description": "Birth year (optional)"},
                        "tags": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Tags associated with the entity"
                        }
                    },
                    "required": ["name", "summary"]
                }
            },
            {
                "name": "findEvents",
                "description": "Return all events whose key starts with the given prefix.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "prefix": {"type": "string", "description": "The prefix to search for"}
                    },
                    "required": ["prefix"]
                }
            },
            {
                "name": "addEvent",
                "description": "Add a new event to the memory store.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Event ID (optional, will be generated if not provided)"},
                        "timestamp": {"type": "string", "description": "Event timestamp (optional, defaults to now)"},
                        "description": {"type": "string", "description": "Event description"},
                        "category": {"type": "string", "description": "Event category"}
                    },
                    "required": ["description", "category"]
                }
            },
            {
                "name": "ingestRepo",
                "description": "Recursively parse a repository's source files (.rs, .py, .ts, .tsx) into ASTs and index every declaration symbol (functions, structs, classes, traits, types, etc.) into the ART. Each symbol is written under both a primary key (location-prefixed) and an inverted key (name-prefixed) so prefix scans can answer 'everything in this file/dir/repo' as well as 'all functions named X across all repos'.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Absolute path to the repository root"},
                        "repo_id": {"type": "string", "description": "Identifier used as the leading segment of every key for this repo (default: directory basename)"}
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "findSymbols",
                "description": "Prefix-scan the AST symbol index. Use 'pri\\u0001<repo>\\u0001<path>\\u0001' for everything in a file, 'pri\\u0001<repo>\\u0001' for an entire repo, or 'sym\\u0001<kind>\\u0001<name>\\u0001' to find every symbol of a given kind+name across all repos.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "prefix": {"type": "string", "description": "Byte-prefix to search for (use \\u0001 as segment separator)"},
                        "limit": {"type": "integer", "description": "Maximum number of results (default: 100)"}
                    },
                    "required": ["prefix"]
                }
            },
            {
                "name": "deleteRepo",
                "description": "Remove every symbol entry (both primary and inverted) for the given repo_id. Useful before re-ingesting a repo to avoid stale entries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": {"type": "string", "description": "The repo_id used during ingestion"}
                    },
                    "required": ["repo_id"]
                }
            }
        ]
    })
}

fn handle_tool_call(memory: &Memory, params: &Value) -> Value {
    let args = &params["arguments"];
    let tool_name = params["name"].as_str().unwrap_or("");

    let result = match tool_name {
        "lookupEntity" => {
            if let Some(name) = args["name"].as_str() {
                if let Some(entity) = memory.lookup_entity(name) {
                    serde_json::to_value(entity).unwrap_or(Value::Null)
                } else {
                    serde_json::json!({"error": format!("Entity not found: {name}")})
                }
            } else {
                serde_json::json!({"error": "Missing name parameter"})
            }
        }

        "addEntity" => {
            if let (Some(name), Some(summary)) = (args["name"].as_str(), args["summary"].as_str()) {
                let entity = Entity {
                    name: name.to_string(),
                    summary: summary.to_string(),
                    born: args["born"].as_str().map(|s| s.to_string()),
                    tags: args["tags"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                if memory.add_entity(entity) {
                    serde_json::json!({"success": true, "message": "Entity added successfully"})
                } else {
                    serde_json::json!({
                        "error": "Entity name contains an interior NUL byte and cannot be used as a key"
                    })
                }
            } else {
                serde_json::json!({"error": "Missing required parameters"})
            }
        }

        "findEvents" => {
            if let Some(prefix) = args["prefix"].as_str() {
                let events = memory.find_events(prefix);
                serde_json::to_value(events).unwrap_or(Value::Null)
            } else {
                serde_json::json!({"error": "Missing prefix parameter"})
            }
        }

        "addEvent" => {
            if let (Some(description), Some(category)) =
                (args["description"].as_str(), args["category"].as_str())
            {
                let event = Event {
                    id: args["id"].as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                        format!(
                            "{}:{}",
                            Utc::now().format("%Y-%m-%d"),
                            category.replace(' ', "-").to_lowercase()
                        )
                    }),
                    timestamp: args["timestamp"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                    description: description.to_string(),
                    category: category.to_string(),
                };
                if memory.add_event(event) {
                    serde_json::json!({"success": true, "message": "Event added successfully"})
                } else {
                    serde_json::json!({
                        "error": "Event id contains an interior NUL byte and cannot be used as a key"
                    })
                }
            } else {
                serde_json::json!({"error": "Missing required parameters"})
            }
        }

        "ingestRepo" => {
            if let Some(path_str) = args["path"].as_str() {
                let path = PathBuf::from(path_str);
                let repo_id = args["repo_id"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("repo")
                            .to_string()
                    });
                let stats = ingest::ingest_repo(memory, &repo_id, &path);
                serde_json::to_value(stats).unwrap_or(Value::Null)
            } else {
                serde_json::json!({"error": "Missing path parameter"})
            }
        }

        "findSymbols" => {
            if let Some(prefix) = args["prefix"].as_str() {
                let limit = args["limit"].as_u64().unwrap_or(100) as usize;
                let hits = memory.find_symbols(prefix, limit);
                serde_json::to_value(hits).unwrap_or(Value::Null)
            } else {
                serde_json::json!({"error": "Missing prefix parameter"})
            }
        }

        "deleteRepo" => {
            if let Some(repo_id) = args["repo_id"].as_str() {
                let removed = memory.delete_repo_symbols(repo_id);
                serde_json::json!({"success": true, "removed": removed})
            } else {
                serde_json::json!({"error": "Missing repo_id parameter"})
            }
        }

        _ => serde_json::json!({"error": format!("Unknown tool: {tool_name}")}),
    };

    serde_json::json!({
        "content": [{"type": "text", "text": result.to_string()}]
    })
}

/// Dispatch a single JSON-RPC request. Returns `None` for notifications
/// (which by spec must produce no response).
pub fn dispatch(memory: &Memory, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
    let response_id = request.id.clone()?; // notifications have no id and need no response

    let response = match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: response_id,
            result: Some(serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
            })),
            error: None,
        },

        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: response_id,
            result: Some(tools_list_payload()),
            error: None,
        },

        "tools/call" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: response_id,
            result: Some(handle_tool_call(memory, &request.params)),
            error: None,
        },

        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: response_id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
            }),
        },
    };

    Some(response)
}

/// Returns true for messages that have no id (notifications) — used by transports
/// that need to know whether to send back HTTP 202 vs a body.
pub fn is_notification(request: &JsonRpcRequest) -> bool {
    request.id.is_none()
}
