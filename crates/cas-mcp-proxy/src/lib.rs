pub mod config;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use rmcp::model::Tool;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::RwLock;

use config::ServerConfig;

/// Result from executing MCP tool calls.
pub struct ExecuteResult {
    /// Text output from the execution.
    pub text: String,
    /// Images returned by the execution.
    pub images: Vec<ImageResult>,
}

/// An image returned from MCP tool execution.
pub struct ImageResult {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type (e.g., "image/png").
    pub mime_type: String,
}

/// A catalog entry describing a tool from an upstream MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

type McpClientService = RunningService<rmcp::RoleClient, ()>;

/// A connected upstream MCP server with its tool catalog.
struct ConnectedServer {
    service: McpClientService,
    tools: Vec<Tool>,
    config: ServerConfig,
}

/// Engine that proxies tool calls to upstream MCP servers.
pub struct ProxyEngine {
    servers: RwLock<HashMap<String, ConnectedServer>>,
}

impl ProxyEngine {
    /// Create a proxy engine by connecting to all configured upstream servers.
    ///
    /// Connection failures are logged and skipped — the engine starts with
    /// whatever servers connected successfully.
    pub async fn from_configs(configs: HashMap<String, ServerConfig>) -> Result<Self> {
        let mut servers = HashMap::new();

        for (name, config) in configs {
            match connect_server(&name, &config).await {
                Ok(connected) => {
                    eprintln!(
                        "[proxy] Connected to '{}' ({} tools)",
                        name,
                        connected.tools.len()
                    );
                    servers.insert(name, connected);
                }
                Err(e) => {
                    eprintln!("[proxy] Failed to connect to '{}': {e:#}", name);
                }
            }
        }

        Ok(Self {
            servers: RwLock::new(servers),
        })
    }

    /// Search across all upstream tool catalogs.
    ///
    /// The `query` parameter supports:
    /// - Plain keywords: case-insensitive substring match on tool name + description
    /// - `server:name` prefix: filter to a specific server before matching keywords
    /// - Empty query: returns all tools
    ///
    /// Results are returned as a JSON array of matching tools with server, name,
    /// description, and input_schema fields. If `max_length` is set, the JSON
    /// output is truncated to that many bytes.
    pub async fn search(&self, query: &str, max_length: Option<usize>) -> Result<Value> {
        let servers = self.servers.read().await;

        // Parse optional server: prefix
        let (server_filter, keywords) = parse_search_query(query);

        let mut results: Vec<SearchResult> = Vec::new();

        for (server_name, connected) in servers.iter() {
            // Apply server filter if present
            if let Some(ref filter) = server_filter {
                if !server_name.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            for tool in &connected.tools {
                if matches_keywords(tool, &keywords) {
                    results.push(SearchResult {
                        server: server_name.clone(),
                        name: tool.name.to_string(),
                        description: tool.description.as_ref().map(|d| d.to_string()),
                        input_schema: serde_json::to_value(&*tool.input_schema)
                            .unwrap_or_default(),
                    });
                }
            }
        }

        let mut json = serde_json::to_string_pretty(&results)?;
        if let Some(max) = max_length {
            if json.len() > max {
                json.truncate(max);
            }
        }

        Ok(Value::String(json))
    }

    /// Execute tool calls across upstream MCP servers.
    ///
    /// The `code` parameter supports two formats:
    ///
    /// **JSON dispatch** (preferred):
    /// ```json
    /// { "server": "github", "tool": "list_issues", "args": { "repo": "myorg/app" } }
    /// ```
    ///
    /// **Batch (parallel)** — array of calls:
    /// ```json
    /// [
    ///   { "server": "github", "tool": "list_issues", "args": { "repo": "myorg/app" } },
    ///   { "server": "sentry", "tool": "list_errors", "args": { "project": "backend" } }
    /// ]
    /// ```
    ///
    /// **Dot-call syntax** (fallback):
    /// ```text
    /// server.tool_name({ "param": "value" })
    /// ```
    pub async fn execute(&self, code: &str, max_length: Option<usize>) -> Result<ExecuteResult> {
        let calls = parse_dispatch(code)?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut images: Vec<ImageResult> = Vec::new();

        if calls.len() == 1 {
            let call = &calls[0];
            let result = self.call_tool_raw(&call.server, &call.tool, call.args.clone()).await?;
            collect_result(&result, &mut text_parts, &mut images);
        } else {
            // Execute in parallel
            let futures: Vec<_> = calls
                .iter()
                .map(|call| self.call_tool_raw(&call.server, &call.tool, call.args.clone()))
                .collect();

            let results = futures::future::join_all(futures).await;

            for (i, result) in results.into_iter().enumerate() {
                match result {
                    Ok(result) => collect_result(&result, &mut text_parts, &mut images),
                    Err(e) => {
                        text_parts.push(format!(
                            "[{}.{} error]: {e}",
                            calls[i].server, calls[i].tool
                        ));
                    }
                }
            }
        }

        let mut text = text_parts.join("\n\n");
        if let Some(max) = max_length {
            if text.len() > max {
                text.truncate(max);
            }
        }

        Ok(ExecuteResult { text, images })
    }

    /// Return the total number of tools across all connected servers.
    pub async fn tool_count(&self) -> usize {
        let servers = self.servers.read().await;
        servers.values().map(|s| s.tools.len()).sum()
    }

    /// Return catalog entries grouped by server name.
    pub async fn catalog_entries_by_server(&self) -> HashMap<String, Vec<CatalogEntry>> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, connected)| {
                let entries = connected
                    .tools
                    .iter()
                    .map(|tool| CatalogEntry {
                        name: tool.name.to_string(),
                        description: tool.description.as_ref().map(|d| d.to_string()),
                        input_schema: serde_json::to_value(&*tool.input_schema)
                            .unwrap_or_default(),
                    })
                    .collect();
                (name.clone(), entries)
            })
            .collect()
    }

    /// Reload with new server configurations.
    ///
    /// Compares against current connections:
    /// - Removes servers no longer in config
    /// - Connects newly added servers
    /// - Reconnects servers whose config changed
    /// - Leaves unchanged servers connected
    pub async fn reload(&self, configs: HashMap<String, ServerConfig>) -> Result<()> {
        let mut servers = self.servers.write().await;

        // Remove servers no longer in config
        let current_names: Vec<String> = servers.keys().cloned().collect();
        for name in &current_names {
            if !configs.contains_key(name) {
                if let Some(removed) = servers.remove(name) {
                    let _ = removed.service.cancel().await;
                    eprintln!("[proxy] Disconnected '{name}'");
                }
            }
        }

        // Connect new servers and reconnect changed ones
        for (name, config) in configs {
            // Check if config changed for existing server
            if let Some(existing) = servers.get(&name) {
                if existing.config == config {
                    continue; // No change, keep existing connection
                }
                // Config changed — disconnect old, will reconnect below
                if let Some(removed) = servers.remove(&name) {
                    let _ = removed.service.cancel().await;
                    eprintln!("[proxy] Config changed for '{name}', reconnecting...");
                }
            }

            match connect_server(&name, &config).await {
                Ok(connected) => {
                    eprintln!(
                        "[proxy] Connected to '{}' ({} tools)",
                        name,
                        connected.tools.len()
                    );
                    servers.insert(name, connected);
                }
                Err(e) => {
                    eprintln!("[proxy] Failed to connect to '{}': {e:#}", name);
                }
            }
        }

        Ok(())
    }

    /// Call a tool on a specific server by name.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> Result<Value> {
        use rmcp::model::CallToolRequestParams;

        let servers = self.servers.read().await;
        let server = servers
            .get(server_name)
            .with_context(|| format!("server '{server_name}' not connected"))?;

        let result = server
            .service
            .call_tool(CallToolRequestParams {
                name: tool_name.to_string().into(),
                arguments,
                meta: None,
                task: None,
            })
            .await
            .with_context(|| format!("tool call '{tool_name}' on '{server_name}' failed"))?;

        serde_json::to_value(result).context("failed to serialize tool result")
    }

    /// Call a tool and return the raw rmcp result (for internal use by execute).
    async fn call_tool_raw(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> Result<rmcp::model::CallToolResult> {
        use rmcp::model::CallToolRequestParams;

        let servers = self.servers.read().await;
        let server = servers.get(server_name).with_context(|| {
            let available: Vec<&str> = servers.keys().map(|s| s.as_str()).collect();
            format!(
                "server '{}' not connected. Available: {}",
                server_name,
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                }
            )
        })?;

        server
            .service
            .call_tool(CallToolRequestParams {
                name: tool_name.to_string().into(),
                arguments,
                meta: None,
                task: None,
            })
            .await
            .with_context(|| format!("tool call '{tool_name}' on '{server_name}' failed"))
    }

    /// Gracefully shut down all connected servers.
    pub async fn shutdown(&self) {
        let mut servers = self.servers.write().await;
        for (name, server) in servers.drain() {
            if let Err(e) = server.service.cancel().await {
                eprintln!("[proxy] Error shutting down '{name}': {e}");
            }
        }
    }
}

/// Connect to a single upstream MCP server and discover its tools.
async fn connect_server(name: &str, config: &ServerConfig) -> Result<ConnectedServer> {
    use rmcp::service::ServiceExt;

    let service: McpClientService = match config {
        ServerConfig::Stdio { command, args, env } => {
            let cmd = Command::new(command);
            let env_clone = env.clone();
            let args_clone = args.clone();
            let transport = TokioChildProcess::new(cmd.configure(move |cmd| {
                cmd.args(&args_clone);
                for (k, v) in &env_clone {
                    cmd.env(k, v);
                }
            }))
            .with_context(|| format!("failed to spawn stdio process for '{name}'"))?;

            ().serve(transport)
                .await
                .with_context(|| format!("failed to initialize MCP client for '{name}'"))?
        }

        ServerConfig::Http { url, auth, .. } => {
            use rmcp::transport::StreamableHttpClientTransport;
            use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

            let mut cfg = StreamableHttpClientTransportConfig::default();
            cfg.uri = Arc::from(url.as_str());
            if let Some(auth_token) = auth {
                cfg.auth_header = Some(format!("Bearer {auth_token}"));
            }

            let transport = StreamableHttpClientTransport::from_config(cfg);

            ().serve(transport)
                .await
                .with_context(|| format!("failed to connect HTTP MCP client for '{name}'"))?
        }

        ServerConfig::Sse { url, auth, .. } => {
            use rmcp::transport::StreamableHttpClientTransport;
            use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

            let mut cfg = StreamableHttpClientTransportConfig::default();
            cfg.uri = Arc::from(url.as_str());
            if let Some(auth_token) = auth {
                cfg.auth_header = Some(format!("Bearer {auth_token}"));
            }

            let transport = StreamableHttpClientTransport::from_config(cfg);

            ().serve(transport)
                .await
                .with_context(|| format!("failed to connect SSE MCP client for '{name}'"))?
        }
    };

    // Discover tools from the server
    let tools_result = service
        .list_tools(Default::default())
        .await
        .with_context(|| format!("failed to list tools from '{name}'"))?;

    Ok(ConnectedServer {
        service,
        tools: tools_result.tools,
        config: config.clone(),
    })
}

/// A search result entry including the server name.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchResult {
    server: String,
    name: String,
    description: Option<String>,
    input_schema: Value,
}

/// Parse a search query into an optional server filter and keyword tokens.
///
/// Supports `server:name keyword1 keyword2` syntax.
fn parse_search_query(query: &str) -> (Option<String>, Vec<String>) {
    let mut server_filter = None;
    let mut keywords = Vec::new();

    for token in query.split_whitespace() {
        if let Some(server) = token.strip_prefix("server:") {
            server_filter = Some(server.to_string());
        } else {
            keywords.push(token.to_lowercase());
        }
    }

    (server_filter, keywords)
}

/// A parsed tool call dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCall {
    server: String,
    tool: String,
    #[serde(default)]
    args: Option<serde_json::Map<String, Value>>,
}

/// Parse the `code` parameter into one or more tool calls.
///
/// Tries JSON dispatch first, then falls back to dot-call syntax.
fn parse_dispatch(code: &str) -> Result<Vec<ToolCall>> {
    let trimmed = code.trim();

    // Try JSON array
    if trimmed.starts_with('[') {
        let calls: Vec<ToolCall> = serde_json::from_str(trimmed)
            .context("failed to parse batch dispatch as JSON array")?;
        if calls.is_empty() {
            anyhow::bail!("empty dispatch array");
        }
        return Ok(calls);
    }

    // Try JSON object
    if trimmed.starts_with('{') {
        let call: ToolCall = serde_json::from_str(trimmed)
            .context("failed to parse dispatch as JSON object")?;
        return Ok(vec![call]);
    }

    // Fall back to dot-call syntax: server.tool_name({ ... })
    parse_dot_syntax(trimmed)
}

/// Parse `server.tool_name({ "param": "value" })` syntax.
fn parse_dot_syntax(code: &str) -> Result<Vec<ToolCall>> {
    let dot_pos = code
        .find('.')
        .context("invalid syntax: expected 'server.tool(args)' or JSON dispatch.\n\nExamples:\n  github.list_issues({\"repo\": \"myorg/app\"})\n  {\"server\": \"github\", \"tool\": \"list_issues\", \"args\": {\"repo\": \"myorg/app\"}}")?;

    let server = &code[..dot_pos];
    let rest = &code[dot_pos + 1..];

    // Find the tool name (everything before the first '(')
    let paren_pos = rest.find('(');

    let (tool, args) = if let Some(pos) = paren_pos {
        let tool_name = &rest[..pos];
        let args_str = rest[pos..].trim();

        // Strip surrounding parens
        let args_inner = args_str
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(args_str)
            .trim();

        let args = if args_inner.is_empty() {
            None
        } else {
            let parsed: serde_json::Map<String, Value> = serde_json::from_str(args_inner)
                .with_context(|| format!("failed to parse arguments as JSON: {args_inner}"))?;
            Some(parsed)
        };

        (tool_name.to_string(), args)
    } else {
        (rest.trim().to_string(), None)
    };

    Ok(vec![ToolCall {
        server: server.to_string(),
        tool,
        args,
    }])
}

/// Extract text and images from an rmcp CallToolResult.
fn collect_result(
    result: &rmcp::model::CallToolResult,
    text_parts: &mut Vec<String>,
    images: &mut Vec<ImageResult>,
) {
    use rmcp::model::RawContent;

    for content in &result.content {
        match &content.raw {
            RawContent::Text(t) => {
                text_parts.push(t.text.clone());
            }
            RawContent::Image(img) => {
                images.push(ImageResult {
                    data: img.data.clone(),
                    mime_type: img.mime_type.clone(),
                });
            }
            _ => {
                // Resource, Audio, ResourceLink — serialize as JSON text
                if let Ok(json) = serde_json::to_string_pretty(&content) {
                    text_parts.push(json);
                }
            }
        }
    }
}

/// Check if a tool matches all keyword tokens (case-insensitive substring on name + description).
fn matches_keywords(tool: &Tool, keywords: &[String]) -> bool {
    if keywords.is_empty() {
        return true;
    }

    let name_lower = tool.name.to_lowercase();
    let desc_lower = tool
        .description
        .as_ref()
        .map(|d| d.to_lowercase())
        .unwrap_or_default();

    keywords.iter().all(|kw| {
        name_lower.contains(kw.as_str()) || desc_lower.contains(kw.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Tool;
    use std::borrow::Cow;
    use std::sync::Arc;

    fn make_tool(name: &str, description: &str) -> Tool {
        Tool {
            name: Cow::Owned(name.to_string()),
            title: None,
            description: Some(Cow::Owned(description.to_string())),
            input_schema: Arc::new(serde_json::Map::new()),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
            execution: None,
        }
    }

    #[test]
    fn parse_query_plain_keywords() {
        let (server, kw) = parse_search_query("screenshot capture");
        assert!(server.is_none());
        assert_eq!(kw, vec!["screenshot", "capture"]);
    }

    #[test]
    fn parse_query_with_server_filter() {
        let (server, kw) = parse_search_query("server:github issue create");
        assert_eq!(server, Some("github".to_string()));
        assert_eq!(kw, vec!["issue", "create"]);
    }

    #[test]
    fn parse_query_empty() {
        let (server, kw) = parse_search_query("");
        assert!(server.is_none());
        assert!(kw.is_empty());
    }

    #[test]
    fn matches_keywords_empty_matches_all() {
        let tool = make_tool("anything", "some description");
        assert!(matches_keywords(&tool, &[]));
    }

    #[test]
    fn matches_keywords_name_match() {
        let tool = make_tool("take_screenshot", "Captures a screenshot");
        let keywords = vec!["screenshot".to_string()];
        assert!(matches_keywords(&tool, &keywords));
    }

    #[test]
    fn matches_keywords_description_match() {
        let tool = make_tool("capture", "Takes a screenshot of the page");
        let keywords = vec!["screenshot".to_string()];
        assert!(matches_keywords(&tool, &keywords));
    }

    #[test]
    fn matches_keywords_case_insensitive() {
        let tool = make_tool("TakeScreenshot", "CAPTURES A SCREENSHOT");
        let keywords = vec!["screenshot".to_string()];
        assert!(matches_keywords(&tool, &keywords));
    }

    #[test]
    fn matches_keywords_all_must_match() {
        let tool = make_tool("create_issue", "Create a GitHub issue");
        let keywords = vec!["create".to_string(), "issue".to_string()];
        assert!(matches_keywords(&tool, &keywords));

        let keywords_no_match = vec!["create".to_string(), "screenshot".to_string()];
        assert!(!matches_keywords(&tool, &keywords_no_match));
    }

    #[test]
    fn matches_keywords_no_match() {
        let tool = make_tool("list_files", "List files in a directory");
        let keywords = vec!["screenshot".to_string()];
        assert!(!matches_keywords(&tool, &keywords));
    }

    // ── Dispatch parsing tests ──────────────────────────────────────

    #[test]
    fn parse_dispatch_json_single() {
        let calls =
            parse_dispatch(r#"{"server": "github", "tool": "list_issues", "args": {"repo": "myorg/app"}}"#)
                .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].server, "github");
        assert_eq!(calls[0].tool, "list_issues");
        assert!(calls[0].args.is_some());
        assert_eq!(calls[0].args.as_ref().unwrap()["repo"], "myorg/app");
    }

    #[test]
    fn parse_dispatch_json_batch() {
        let calls = parse_dispatch(
            r#"[
                {"server": "github", "tool": "list_issues", "args": {"repo": "app"}},
                {"server": "sentry", "tool": "list_errors", "args": {"project": "be"}}
            ]"#,
        )
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].server, "github");
        assert_eq!(calls[1].server, "sentry");
    }

    #[test]
    fn parse_dispatch_dot_syntax_with_args() {
        let calls =
            parse_dispatch(r#"github.list_issues({"repo": "myorg/app"})"#).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].server, "github");
        assert_eq!(calls[0].tool, "list_issues");
        assert!(calls[0].args.is_some());
    }

    #[test]
    fn parse_dispatch_dot_syntax_no_args() {
        let calls = parse_dispatch("github.list_repos()").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].server, "github");
        assert_eq!(calls[0].tool, "list_repos");
        assert!(calls[0].args.is_none());
    }

    #[test]
    fn parse_dispatch_dot_syntax_no_parens() {
        let calls = parse_dispatch("github.list_repos").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].server, "github");
        assert_eq!(calls[0].tool, "list_repos");
        assert!(calls[0].args.is_none());
    }

    #[test]
    fn parse_dispatch_invalid_no_dot() {
        let result = parse_dispatch("just_a_word");
        assert!(result.is_err());
    }

    #[test]
    fn parse_dispatch_json_no_args() {
        let calls =
            parse_dispatch(r#"{"server": "github", "tool": "list_repos"}"#).unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].args.is_none());
    }
}
