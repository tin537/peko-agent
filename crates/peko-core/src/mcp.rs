use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn, error};

use crate::tool::{Tool, ToolResult};

/// MCP client that connects to an MCP server via stdio or HTTP.
/// Discovers tools from the server and makes them available to the agent.
pub struct McpClient {
    name: String,
    transport: McpTransport,
    tools: Vec<McpToolDef>,
    request_id: u64,
}

enum McpTransport {
    Stdio {
        child: Child,
        stdin: tokio::process::ChildStdin,
        reader: BufReader<tokio::process::ChildStdout>,
    },
    Http {
        client: reqwest::Client,
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl McpClient {
    /// Connect to an MCP server via stdio (spawns a child process)
    pub async fn connect_stdio(config: &McpServerConfig) -> anyhow::Result<Self> {
        let cmd = config.command.as_deref()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' missing 'command'", config.name))?;

        let mut command = Command::new(cmd);
        command.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (k, v) in &config.env {
            command.env(k, v);
        }

        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let reader = BufReader::new(stdout);

        let mut client = Self {
            name: config.name.clone(),
            transport: McpTransport::Stdio { child, stdin, reader },
            tools: Vec::new(),
            request_id: 0,
        };

        // Initialize MCP session
        client.initialize().await?;

        // Discover tools
        client.discover_tools().await?;

        info!(server = %config.name, tools = client.tools.len(), "MCP server connected (stdio)");
        Ok(client)
    }

    /// Connect to an MCP server via HTTP
    pub async fn connect_http(config: &McpServerConfig) -> anyhow::Result<Self> {
        let url = config.url.as_deref()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' missing 'url'", config.name))?;

        let client = reqwest::Client::new();

        let mut mcp = Self {
            name: config.name.clone(),
            transport: McpTransport::Http {
                client,
                url: url.to_string(),
            },
            tools: Vec::new(),
            request_id: 0,
        };

        mcp.initialize().await?;
        mcp.discover_tools().await?;

        info!(server = %config.name, tools = mcp.tools.len(), "MCP server connected (HTTP)");
        Ok(mcp)
    }

    /// Connect based on config (auto-detect stdio vs HTTP)
    pub async fn connect(config: &McpServerConfig) -> anyhow::Result<Self> {
        if config.url.is_some() {
            Self::connect_http(config).await
        } else {
            Self::connect_stdio(config).await
        }
    }

    async fn next_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }

    async fn send_request(&mut self, method: &str, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id().await;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let request_str = serde_json::to_string(&request)? + "\n";

        match &mut self.transport {
            McpTransport::Stdio { stdin, reader, .. } => {
                stdin.write_all(request_str.as_bytes()).await?;
                stdin.flush().await?;

                let mut line = String::new();
                reader.read_line(&mut line).await?;
                let response: serde_json::Value = serde_json::from_str(&line)?;

                if let Some(error) = response.get("error") {
                    anyhow::bail!("MCP error: {}", error);
                }

                Ok(response.get("result").cloned().unwrap_or(json!(null)))
            }
            McpTransport::Http { client, url } => {
                let resp = client.post(url.as_str())
                    .json(&request)
                    .send().await?
                    .json::<serde_json::Value>().await?;

                if let Some(error) = resp.get("error") {
                    anyhow::bail!("MCP error: {}", error);
                }

                Ok(resp.get("result").cloned().unwrap_or(json!(null)))
            }
        }
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let result = self.send_request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "peko-agent",
                "version": "0.1.0"
            }
        })).await?;

        info!(server = %self.name, "MCP initialized: {:?}", result.get("serverInfo"));
        Ok(())
    }

    async fn discover_tools(&mut self) -> anyhow::Result<()> {
        let result = self.send_request("tools/list", json!({})).await?;

        if let Some(tools) = result.get("tools").and_then(|t| t.as_array()) {
            self.tools = tools.iter().filter_map(|t| {
                serde_json::from_value(t.clone()).ok()
            }).collect();
        }

        Ok(())
    }

    /// Call a tool on the MCP server
    pub async fn call_tool(&mut self, name: &str, args: serde_json::Value) -> anyhow::Result<String> {
        let result = self.send_request("tools/call", json!({
            "name": name,
            "arguments": args,
        })).await?;

        // Extract text content from MCP response
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            let texts: Vec<String> = content.iter()
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect();
            Ok(texts.join("\n"))
        } else {
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }

    pub fn tool_definitions(&self) -> &[McpToolDef] {
        &self.tools
    }

    pub fn server_name(&self) -> &str {
        &self.name
    }
}

/// Wrapper that makes an MCP tool usable as a peko Tool
pub struct McpToolAdapter {
    server_name: String,
    tool_def: McpToolDef,
    client: Arc<Mutex<McpClient>>,
}

impl McpToolAdapter {
    pub fn new(server_name: &str, tool_def: McpToolDef, client: Arc<Mutex<McpClient>>) -> Self {
        Self {
            server_name: server_name.to_string(),
            tool_def,
            client,
        }
    }
}

impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.tool_def.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool_def.input_schema.clone()
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let client = self.client.clone();
        let name = self.tool_def.name.clone();
        Box::pin(async move {
            let mut c = client.lock().await;
            match c.call_tool(&name, args).await {
                Ok(result) => Ok(ToolResult::success(result)),
                Err(e) => Ok(ToolResult::error(format!("MCP tool error: {}", e))),
            }
        })
    }
}

/// Connect to all configured MCP servers and register their tools
pub async fn register_mcp_tools(
    configs: &[McpServerConfig],
    registry: &mut crate::tool::ToolRegistry,
) -> Vec<Arc<Mutex<McpClient>>> {
    let mut clients = Vec::new();

    for config in configs {
        match McpClient::connect(config).await {
            Ok(client) => {
                let client = Arc::new(Mutex::new(client));
                let tool_defs: Vec<McpToolDef>;

                {
                    let c = client.lock().await;
                    tool_defs = c.tool_definitions().to_vec();
                    info!(
                        server = %c.server_name(),
                        tools = tool_defs.len(),
                        "MCP tools discovered"
                    );
                }

                for def in tool_defs {
                    let adapter = McpToolAdapter::new(
                        &config.name,
                        def,
                        client.clone(),
                    );
                    registry.register(adapter);
                }

                clients.push(client);
            }
            Err(e) => {
                error!(server = %config.name, error = %e, "failed to connect MCP server");
            }
        }
    }

    clients
}
