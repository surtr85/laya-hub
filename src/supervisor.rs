use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::{bail, Context, Result};
use reqwest::Client as HttpClient;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::config::{AppConfig, McpServerConfig};

pub struct ProcessSupervisor {
    config: Arc<AppConfig>,
    active_stdio: Arc<Mutex<HashMap<String, StdioProcess>>>,
    http_client: HttpClient,
    tools_cache: Arc<Mutex<HashMap<String, Value>>>,
}

struct StdioProcess {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    last_used: Instant,
}

impl ProcessSupervisor {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self {
            config,
            active_stdio: Arc::new(Mutex::new(HashMap::new())),
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            tools_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Query tools/list for a specific server, with thread-safe caching
    pub async fn list_tools(&self, server_id: &str, force_refresh: bool) -> Result<Value> {
        if !force_refresh {
            let cache = self.tools_cache.lock().await;
            if let Some(cached) = cache.get(server_id) {
                return Ok(cached.clone());
            }
        }

        let rpc_call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/list",
            "params": {}
        });

        let resp = self.call_server(server_id, rpc_call).await?;

        let tools = resp
            .get("result")
            .and_then(|r| r.get("tools"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));

        {
            let mut cache = self.tools_cache.lock().await;
            cache.insert(server_id.to_string(), tools.clone());
        }

        Ok(tools)
    }

    /// Forward a JSON-RPC request to a target MCP server
    pub async fn call_server(&self, server_id: &str, req: Value) -> Result<Value> {
        let server_cfg = self
            .config
            .mcp_servers
            .get(server_id)
            .with_context(|| format!("Unknown MCP server '{}'", server_id))?;

        match server_cfg {
            McpServerConfig::Stdio {
                command,
                args,
                env,
                cwd,
                ..
            } => {
                self.call_stdio(server_id, command, args, env, cwd.as_deref(), req)
                    .await
            }
            McpServerConfig::Http { url, headers, .. } => {
                self.call_http(url, headers, req).await
            }
        }
    }

    async fn call_http(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        payload: Value,
    ) -> Result<Value> {
        let mut req_builder = self.http_client.post(url).json(&payload);
        for (k, v) in headers {
            if !v.trim().is_empty() {
                req_builder = req_builder.header(k, v);
            }
        }

        let resp = req_builder
            .send()
            .await
            .with_context(|| format!("HTTP request to {} failed", url))?;

        let body: Value = resp
            .json()
            .await
            .context("Failed to parse HTTP JSON-RPC response")?;

        Ok(body)
    }

    async fn call_stdio(
        &self,
        server_id: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&str>,
        payload: Value,
    ) -> Result<Value> {
        let mut map = self.active_stdio.lock().await;

        let needs_spawn = match map.get_mut(server_id) {
            Some(proc) => {
                // Check if still running
                match proc.child.try_wait() {
                    Ok(None) => false, // still running
                    _ => true,
                }
            }
            None => true,
        };

        if needs_spawn {
            info!("Spawning on-demand MCP server '{}': {}", server_id, command);
            let mut cmd = Command::new(command);
            cmd.args(args);
            cmd.envs(env);
            if let Some(dir) = cwd {
                cmd.current_dir(dir);
            }
            cmd.stdin(Stdio::piped());
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::inherit());

            let mut child = cmd
                .spawn()
                .with_context(|| format!("Failed to spawn child command '{}'", command))?;

            let stdin = child
                .stdin
                .take()
                .context("Child process stdin not available")?;
            let stdout = child
                .stdout
                .take()
                .context("Child process stdout not available")?;

            let mut proc = StdioProcess {
                child,
                stdin,
                reader: BufReader::new(stdout),
                last_used: Instant::now(),
            };

            // Initial handshake if needed by MCP protocol (initialize)
            Self::do_initialize_handshake(&mut proc, server_id).await?;

            map.insert(server_id.to_string(), proc);
        }

        let proc = map
            .get_mut(server_id)
            .context("Failed to acquire process after spawn")?;

        proc.last_used = Instant::now();

        // Write line
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        proc.stdin.write_all(line.as_bytes()).await?;
        proc.stdin.flush().await?;

        // Read line response with robust parser
        let expected_id = payload.get("id");
        let resp = Self::read_jsonrpc_response(
            &mut proc.reader,
            server_id,
            expected_id,
            Duration::from_secs(60),
        )
        .await?;

        Ok(resp)
    }

    async fn read_jsonrpc_response(
        reader: &mut BufReader<ChildStdout>,
        server_id: &str,
        expected_id: Option<&Value>,
        timeout_duration: Duration,
    ) -> Result<Value> {
        let deadline = Instant::now() + timeout_duration;

        loop {
            let now = Instant::now();
            if now >= deadline {
                bail!(
                    "Timeout ({:?}) waiting for JSON-RPC response from MCP server '{}'",
                    timeout_duration,
                    server_id
                );
            }
            let remaining = deadline - now;
            let mut line = String::new();

            let n = match tokio::time::timeout(remaining, reader.read_line(&mut line)).await {
                Ok(res) => res.with_context(|| {
                    format!("Error reading stdout from MCP server '{}'", server_id)
                })?,
                Err(_) => {
                    bail!(
                        "Timeout ({:?}) waiting for JSON-RPC response from MCP server '{}'",
                        timeout_duration,
                        server_id
                    );
                }
            };

            if n == 0 {
                bail!("MCP server '{}' closed connection unexpectedly", server_id);
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Skip lines that do not start with '{' (likely server logs or warnings on stdout)
            if !trimmed.starts_with('{') {
                debug!(
                    "MCP server '{}' emitted non-JSON line on stdout: {}",
                    server_id, trimmed
                );
                continue;
            }

            match serde_json::from_str::<Value>(trimmed) {
                Ok(val) => {
                    // Check if this is an MCP notification (has method and no id)
                    if val.get("method").is_some() && val.get("id").is_none() {
                        debug!(
                            "MCP server '{}' emitted notification: {}",
                            server_id, trimmed
                        );
                        continue;
                    }

                    // If an expected_id was provided, check if it matches
                    if let Some(exp_id) = expected_id {
                        if let Some(resp_id) = val.get("id") {
                            if resp_id != exp_id {
                                debug!(
                                    "MCP server '{}' sent response with mismatched id (got {}, expected {}): {}",
                                    server_id, resp_id, exp_id, trimmed
                                );
                                continue;
                            }
                        }
                    }

                    return Ok(val);
                }
                Err(err) => {
                    debug!(
                        "MCP server '{}' emitted unparseable JSON on stdout: {} ({})",
                        server_id, trimmed, err
                    );
                    continue;
                }
            }
        }
    }

    async fn do_initialize_handshake(proc: &mut StdioProcess, server_id: &str) -> Result<()> {
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "laya-hub",
                    "version": "0.1.0"
                }
            }
        });

        let mut line = serde_json::to_string(&init_req)?;
        line.push('\n');
        proc.stdin.write_all(line.as_bytes()).await?;
        proc.stdin.flush().await?;

        let expected_id = serde_json::json!(1);
        let _init_resp = Self::read_jsonrpc_response(
            &mut proc.reader,
            server_id,
            Some(&expected_id),
            Duration::from_secs(30),
        )
        .await
        .with_context(|| format!("Failed during initialize handshake with '{}'", server_id))?;

        // Send notifications/initialized
        let notify = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let mut notif_line = serde_json::to_string(&notify)?;
        notif_line.push('\n');
        proc.stdin.write_all(notif_line.as_bytes()).await?;
        proc.stdin.flush().await?;

        debug!("MCP handshake complete for '{}'", server_id);
        Ok(())
    }

    /// Background task to clean up idle processes beyond idle_timeout_secs
    pub fn start_idle_sweeper(self: Arc<Self>, idle_secs: u64) {
        tokio::spawn(async move {
            let timeout = Duration::from_secs(idle_secs);
            let mut interval = tokio::time::interval(Duration::from_secs(30));

            loop {
                interval.tick().await;
                let mut map = self.active_stdio.lock().await;
                let mut to_remove = Vec::new();

                for (id, proc) in map.iter_mut() {
                    if proc.last_used.elapsed() > timeout {
                        info!("Shutting down idle MCP server '{}'", id);
                        let _ = proc.child.kill().await;
                        to_remove.push(id.clone());
                    }
                }

                for id in to_remove {
                    map.remove(&id);
                }
            }
        });
    }

    pub async fn active_servers(&self) -> Vec<String> {
        let map = self.active_stdio.lock().await;
        map.keys().cloned().collect()
    }
}
