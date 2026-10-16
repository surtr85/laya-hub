use std::sync::Arc;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

use crate::config::AppConfig;
use crate::scorer::LayaScorer;
use crate::supervisor::ProcessSupervisor;

pub struct McpHandler {
    config: Arc<AppConfig>,
    scorer: Arc<LayaScorer>,
    supervisor: Arc<ProcessSupervisor>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

impl McpHandler {
    pub fn new(
        config: Arc<AppConfig>,
        scorer: Arc<LayaScorer>,
        supervisor: Arc<ProcessSupervisor>,
    ) -> Self {
        Self {
            config,
            scorer,
            supervisor,
        }
    }

    pub async fn run_stdio_loop(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = tokio::io::stdout();

        let mut line = String::new();
        while reader.read_line(&mut line).await? > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                Ok(req) => {
                    let resp = self.handle_request(req).await;
                    if let Some(resp_val) = resp {
                        let mut resp_str = serde_json::to_string(&resp_val)?;
                        resp_str.push('\n');
                        stdout.write_all(resp_str.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                }
                Err(err) => {
                    error!("Invalid JSON-RPC line: {} ({})", trimmed, err);
                }
            }

            line.clear();
        }

        Ok(())
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<Value> {
        let req_id = req.id.clone();

        match req.method.as_str() {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "laya-hub",
                        "version": "0.1.0"
                    },
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    }
                });
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": result
                }))
            }

            "notifications/initialized" => {
                info!("MCP client initialized session");
                None
            }

            "tools/list" => {
                let tools = vec![
                    json!({
                        "name": "hub_route",
                        "description": "Evaluates task via Laya System-1 (<40ms), scores registered MCP servers and skills, and returns matched capabilities along with full skill instructions.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "task": {
                                    "type": "string",
                                    "description": "The user request or task description to route"
                                }
                            },
                            "required": ["task"]
                        }
                    }),
                    json!({
                        "name": "hub_call",
                        "description": "Executes a tool on any registered MCP server on-demand. Spawns and manages server lifecycle transparently.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "server": {
                                    "type": "string",
                                    "description": "Target registered MCP server ID (e.g. github, composio, miku_researcher)"
                                },
                                "tool": {
                                    "type": "string",
                                    "description": "Name of the target tool to call on that server"
                                },
                                "arguments": {
                                    "type": "object",
                                    "description": "Arguments passed to the underlying tool"
                                }
                            },
                            "required": ["server", "tool"]
                        }
                    }),
                    json!({
                        "name": "hub_status",
                        "description": "Returns current supervisor status: loaded skills, registered MCP servers, active running servers, and Laya configuration.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    })
                ];

                Some(json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": { "tools": tools }
                }))
            }

            "tools/call" => {
                let tool_name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));

                let result = match tool_name {
                    "hub_route" => self.handle_hub_route(args).await,
                    "hub_call" => self.handle_hub_call(args).await,
                    "hub_status" => self.handle_hub_status().await,
                    _ => Err(anyhow::anyhow!("Unknown tool '{}'", tool_name)),
                };

                match result {
                    Ok(val) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&val).unwrap_or_default()
                                }
                            ]
                        }
                    })),
                    Err(err) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32603,
                            "message": err.to_string()
                        }
                    })),
                }
            }

            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {
                    "code": -32601,
                    "message": format!("Method '{}' not found", req.method)
                }
            })),
        }
    }

    async fn handle_hub_route(&self, args: Value) -> Result<Value> {
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .context("Parameter 'task' is required for hub_route")?;

        let decision = self.scorer.route_task(task, &self.config).await?;
        Ok(json!(decision))
    }

    async fn handle_hub_call(&self, args: Value) -> Result<Value> {
        let server = args
            .get("server")
            .and_then(|v| v.as_str())
            .context("Parameter 'server' is required for hub_call")?;

        let tool = args
            .get("tool")
            .and_then(|v| v.as_str())
            .context("Parameter 'tool' is required for hub_call")?;

        let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));

        let rpc_call = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": tool_args
            }
        });

        let resp = self.supervisor.call_server(server, rpc_call).await?;
        Ok(resp)
    }

    async fn handle_hub_status(&self) -> Result<Value> {
        let active = self.supervisor.active_servers().await;
        let servers: Vec<String> = self.config.mcp_servers.keys().cloned().collect();
        let skills: Vec<String> = self.config.skills.keys().cloned().collect();

        Ok(json!({
            "laya_engine": {
                "url": self.config.laya.url,
                "threshold": self.config.laya.threshold,
                "idle_timeout_secs": self.config.laya.idle_timeout_secs
            },
            "registered_servers": servers,
            "active_running_servers": active,
            "registered_skills": skills
        }))
    }
}
