//! Simple MCP memory server implementation for ARM64 compatibility
//! 
//! This version provides basic MCP functionality with entity and event management
//! using standard Rust collections for broad compatibility.

use std::{fs, path::PathBuf, sync::Arc, collections::BTreeMap};
use anyhow::{Context, Result};
use clap::Parser;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use chrono::Utc;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(name = "blazing_art_mcp", about = "MCP memory server")]
struct Cli {
    #[arg(long)]
    entities: Option<PathBuf>,
    
    #[arg(long)]
    events: Option<PathBuf>,
    
    #[arg(long, default_value_t = 100)]
    event_limit: usize,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Entity {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Event {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub category: String,
}

struct Memory {
    entities: Arc<RwLock<BTreeMap<String, Entity>>>,
    events: Arc<RwLock<BTreeMap<String, Event>>>,
    event_limit: usize,
}

impl Memory {
    fn new(event_limit: usize) -> Self {
        Self {
            entities: Arc::new(RwLock::new(BTreeMap::new())),
            events: Arc::new(RwLock::new(BTreeMap::new())),
            event_limit,
        }
    }

    fn lookup_entity(&self, name: &str) -> Option<Entity> {
        self.entities.read().get(name).cloned()
    }

    fn add_entity(&self, entity: Entity) {
        self.entities.write().insert(entity.name.clone(), entity);
    }

    fn find_events(&self, prefix: &str) -> Vec<Event> {
        self.events
            .read()
            .range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .take(self.event_limit)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn add_event(&self, event: Event) {
        self.events.write().insert(event.id.clone(), event);
    }

    fn load_entities(&self, path: &PathBuf) -> Result<()> {
        let text = fs::read_to_string(path)?;
        let list: Vec<Entity> = serde_json::from_str(&text)?;
        
        let mut entities = self.entities.write();
        for e in list {
            entities.insert(e.name.clone(), e);
        }
        
        eprintln!("Loaded {} entities", entities.len());
        Ok(())
    }

    fn load_events(&self, path: &PathBuf) -> Result<()> {
        let text = fs::read_to_string(path)?;
        let list: Vec<Event> = serde_json::from_str(&text)?;
        
        let mut events = self.events.write();
        for ev in list {
            events.insert(ev.id.clone(), ev);
        }
        
        eprintln!("Loaded {} events", events.len());
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

async fn handle_request(memory: &Memory, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
    let response_id = request.id.clone();
    
    // Handle notifications (no response needed)
    if response_id.is_none() {
        match request.method.as_str() {
            "notifications/initialized" => {
                eprintln!("Received initialized notification");
                return None;
            }
            _ => {
                eprintln!("Unknown notification: {}", request.method);
                return None;
            }
        }
    }
    
    let response_id = response_id.unwrap();
    
    let response = match request.method.as_str() {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "blazing-art-mcp",
                    "version": "0.1.0"
                }
            });
            
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: response_id,
                result: Some(result),
                error: None,
            }
        }
        
        "tools/list" => {
            let tools = serde_json::json!({
                "tools": [
                    {
                        "name": "lookupEntity",
                        "description": "Retrieve stored information about an entity by exact name.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "The exact name of the entity to look up"
                                }
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
                                "name": {
                                    "type": "string",
                                    "description": "The name of the entity"
                                },
                                "summary": {
                                    "type": "string",
                                    "description": "A summary of the entity"
                                },
                                "born": {
                                    "type": "string",
                                    "description": "Birth year (optional)"
                                },
                                "tags": {
                                    "type": "array",
                                    "items": {
                                        "type": "string"
                                    },
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
                                "prefix": {
                                    "type": "string",
                                    "description": "The prefix to search for"
                                }
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
                                "id": {
                                    "type": "string",
                                    "description": "Event ID (optional, will be generated if not provided)"
                                },
                                "timestamp": {
                                    "type": "string",
                                    "description": "Event timestamp (optional, defaults to now)"
                                },
                                "description": {
                                    "type": "string",
                                    "description": "Event description"
                                },
                                "category": {
                                    "type": "string",
                                    "description": "Event category"
                                }
                            },
                            "required": ["description", "category"]
                        }
                    }
                ]
            });
            
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: response_id,
                result: Some(tools),
                error: None,
            }
        }
        
        "tools/call" => {
            let args = &request.params["arguments"];
            let tool_name = request.params["name"].as_str().unwrap_or("");
            
            let result = match tool_name {
                "lookupEntity" => {
                    if let Some(name) = args["name"].as_str() {
                        if let Some(entity) = memory.lookup_entity(name) {
                            serde_json::to_value(entity).unwrap()
                        } else {
                            serde_json::json!({
                                "error": format!("Entity not found: {}", name)
                            })
                        }
                    } else {
                        serde_json::json!({"error": "Missing name parameter"})
                    }
                }
                
                "addEntity" => {
                    if let (Some(name), Some(summary)) = 
                        (args["name"].as_str(), args["summary"].as_str()) {
                        let entity = Entity {
                            name: name.to_string(),
                            summary: summary.to_string(),
                            born: args["born"].as_str().map(|s| s.to_string()),
                            tags: args["tags"].as_array()
                                .map(|arr| arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect())
                                .unwrap_or_default(),
                        };
                        memory.add_entity(entity);
                        serde_json::json!({
                            "success": true,
                            "message": "Entity added successfully"
                        })
                    } else {
                        serde_json::json!({"error": "Missing required parameters"})
                    }
                }
                
                "findEvents" => {
                    if let Some(prefix) = args["prefix"].as_str() {
                        let events = memory.find_events(prefix);
                        serde_json::to_value(events).unwrap()
                    } else {
                        serde_json::json!({"error": "Missing prefix parameter"})
                    }
                }
                
                "addEvent" => {
                    if let (Some(description), Some(category)) = 
                        (args["description"].as_str(), args["category"].as_str()) {
                        let event = Event {
                            id: args["id"].as_str()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    format!("{}:{}", 
                                        Utc::now().format("%Y-%m-%d"),
                                        category.replace(" ", "-").to_lowercase()
                                    )
                                }),
                            timestamp: args["timestamp"].as_str()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| Utc::now().to_rfc3339()),
                            description: description.to_string(),
                            category: category.to_string(),
                        };
                        memory.add_event(event);
                        serde_json::json!({
                            "success": true,
                            "message": "Event added successfully"
                        })
                    } else {
                        serde_json::json!({"error": "Missing required parameters"})
                    }
                }
                
                _ => serde_json::json!({"error": format!("Unknown tool: {}", tool_name)})
            };
            
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: response_id,
                result: Some(serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": result.to_string()
                        }
                    ]
                })),
                error: None,
            }
        }
        
        _ => {
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: response_id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                }),
            }
        }
    };
    
    Some(response)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    let memory = Memory::new(cli.event_limit);
    
    if let Some(p) = cli.entities.as_ref() {
        memory.load_entities(p).context("loading entities")?;
    }
    if let Some(p) = cli.events.as_ref() {
        memory.load_events(p).context("loading events")?;
    }

    eprintln!("Blazing-ART-MCP Server started (STDIO mode)");
    
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
                break; // EOF
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                
                eprintln!("Received request: {}", trimmed);
                
                match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                    Ok(request) => {
                        if let Some(response) = handle_request(&memory, request).await {
                            let response_str = serde_json::to_string(&response)?;
                            eprintln!("Sending response: {}", response_str);
                            
                            // Handle potential broken pipe errors
                            if let Err(e) = stdout.write_all(response_str.as_bytes()).await {
                                eprintln!("Error writing response: {}", e);
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    eprintln!("Client closed connection");
                                    break;
                                }
                                return Err(e.into());
                            }
                            
                            if let Err(e) = stdout.write_all(b"\n").await {
                                eprintln!("Error writing newline: {}", e);
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    eprintln!("Client closed connection");
                                    break;
                                }
                                return Err(e.into());
                            }
                            
                            if let Err(e) = stdout.flush().await {
                                eprintln!("Error flushing: {}", e);
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    eprintln!("Client closed connection");
                                    break;
                                }
                                return Err(e.into());
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to parse request: {}", e);
                        // Send error response
                        let error_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: serde_json::Value::Null,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32700,
                                message: format!("Parse error: {}", e),
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
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }
    
    eprintln!("MCP server shutting down");
    
    Ok(())
}