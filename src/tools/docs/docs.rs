use crate::tools::item_list;
use std::{future::Future, pin::Pin, sync::Arc};

use mcp_core::{
    handler::{PromptError, ResourceError},
    prompt::Prompt,
    protocol::ServerCapabilities,
    Content, Resource, Tool, ToolError,
};
use mcp_server::router::CapabilitiesBuilder;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use html2md::parse_html;

// Cache for documentation lookups to avoid repeated requests
#[derive(Clone)]
pub struct DocCache {
    cache: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl Default for DocCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DocCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        let cache = self.cache.lock().await;
        cache.get(key).cloned()
    }

    pub async fn set(&self, key: String, value: String) {
        let mut cache = self.cache.lock().await;
        cache.insert(key, value);
    }
}

#[derive(Clone)]
pub struct DocRouter {
    pub client: Client,
    pub cache: DocCache,
}

impl Default for DocRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl DocRouter {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            cache: DocCache::new(),
        }
    }

    // Fetch crate documentation from docs.rs
    async fn lookup_crate(&self, crate_name: String, version: Option<String>) -> Result<String, ToolError> {
        // Check cache first
        let cache_key = if let Some(ver) = &version {
            format!("{}:{}", crate_name, ver)
        } else {
            crate_name.clone()
        };

        if let Some(doc) = self.cache.get(&cache_key).await {
            return Ok(doc);
        }

        // Construct the docs.rs URL for the crate
        let url = if let Some(ver) = version {
            format!("https://docs.rs/crate/{}/{}/", crate_name, ver)
        } else {
            format!("https://docs.rs/crate/{}/", crate_name)
        };

        // Fetch the documentation page
        let response = self.client.get(&url)
            .header("User-Agent", "CrateDocs/0.1.0 (https://github.com/d6e/cratedocs-mcp)")
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to fetch documentation: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(ToolError::ExecutionError(format!(
                "Failed to fetch documentation. Status: {}",
                response.status()
            )));
        }

        let html_body = response.text().await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to read response body: {}", e))
        })?;
        
        // Convert HTML to markdown
        let markdown_body = parse_html(&html_body);

        // Cache the markdown result
        self.cache.set(cache_key, markdown_body.clone()).await;
        
        Ok(markdown_body)
    }

    // Search crates.io for crates matching a query
    async fn search_crates(&self, query: String, limit: Option<u32>) -> Result<String, ToolError> {
        let limit = limit.unwrap_or(10).min(100); // Cap at 100 results
        
        let url = format!("https://crates.io/api/v1/crates?q={}&per_page={}", query, limit);
        
        let response = self.client.get(&url)
            .header("User-Agent", "CrateDocs/0.1.0 (https://github.com/d6e/cratedocs-mcp)")
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to search crates.io: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(ToolError::ExecutionError(format!(
                "Failed to search crates.io. Status: {}",
                response.status()
            )));
        }

        let body = response.text().await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to read response body: {}", e))
        })?;
        
        // Check if response is JSON (API response) or HTML (web page)
        if body.trim().starts_with('{') {
            // This is likely JSON data, return as is
            Ok(body)
        } else {
            // This is likely HTML, convert to markdown
            Ok(parse_html(&body))
        }
    }

    // Get documentation for a specific item in a crate
    async fn lookup_item(&self, crate_name: String, mut item_path: String, version: Option<String>) -> Result<String, ToolError> {
        // Strip crate name prefix from the item path if it exists
        let crate_prefix = format!("{}::", crate_name);
        if item_path.starts_with(&crate_prefix) {
            item_path = item_path[crate_prefix.len()..].to_string();
        }

        // Check cache first
        let cache_key = if let Some(ver) = &version {
            format!("{}:{}:{}", crate_name, ver, item_path)
        } else {
            format!("{}:{}", crate_name, item_path)
        };

        if let Some(doc) = self.cache.get(&cache_key).await {
            return Ok(doc);
        }

        // Process the item path to determine the item type
        // Format: module::path::ItemName
        // Need to split into module path and item name, and guess item type
        let parts: Vec<&str> = item_path.split("::").collect();
        
        if parts.is_empty() {
            return Err(ToolError::InvalidParameters(
                "Invalid item path. Expected format: module::path::ItemName".to_string()
            ));
        }
        
        let item_name = parts.last().unwrap().to_string();
        let module_path = if parts.len() > 1 {
            parts[..parts.len()-1].join("/")
        } else {
            String::new()
        };
        
        // Try different item types (struct, enum, trait, fn)
        let item_types = ["struct", "enum", "trait", "fn", "macro"];
        let mut last_error = None;
        
        for item_type in item_types.iter() {
            // Construct the docs.rs URL for the specific item
            let url = if let Some(ver) = version.clone() {
                if module_path.is_empty() {
                    format!("https://docs.rs/{}/{}/{}/{}.{}.html", crate_name, ver, crate_name, item_type, item_name)
                } else {
                    format!("https://docs.rs/{}/{}/{}/{}/{}.{}.html", crate_name, ver, crate_name, module_path, item_type, item_name)
                }
            } else {
                if module_path.is_empty() {
                    format!("https://docs.rs/{}/latest/{}/{}.{}.html", crate_name, crate_name, item_type, item_name)
                } else {
                    format!("https://docs.rs/{}/latest/{}/{}/{}.{}.html", crate_name, crate_name, module_path, item_type, item_name)
                }
            };
            
            // Try to fetch the documentation page
            let response = match self.client.get(&url)
                .header("User-Agent", "CrateDocs/0.1.0 (https://github.com/d6e/cratedocs-mcp)")
                .send().await {
                Ok(resp) => resp,
                Err(e) => {
                    last_error = Some(e.to_string());
                    continue;
                }
            };
            
            // If found, process and return
            if response.status().is_success() {
                let html_body = response.text().await.map_err(|e| {
                    ToolError::ExecutionError(format!("Failed to read response body: {}", e))
                })?;
                
                // Convert HTML to markdown
                let markdown_body = parse_html(&html_body);
                
                // Cache the markdown result
                self.cache.set(cache_key, markdown_body.clone()).await;
                
                return Ok(markdown_body);
            }
            
            last_error = Some(format!("Status code: {}", response.status()));
        }
        
        // If we got here, none of the item types worked
        Err(ToolError::ExecutionError(format!(
            "Failed to fetch item documentation. No matching item found. Last error: {}",
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        )))
    }
}

impl mcp_server::Router for DocRouter {
    fn name(&self) -> String {
        "rust-docs".to_string()
    }

    fn instructions(&self) -> String {
        "This server provides tools for looking up Rust crate documentation in markdown format. \
        You can search for crates, lookup documentation for specific crates or \
        items within crates. Use these tools to find information about Rust libraries \
        you are not familiar with. All HTML documentation is automatically converted to markdown \
        for better compatibility with language models.".to_string()
    }

    fn capabilities(&self) -> ServerCapabilities {
        CapabilitiesBuilder::new()
            .with_tools(true)
            .with_resources(false, false)
            .with_prompts(false)
            .build()
    }

    fn list_tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "lookup_crate".to_string(),
                "Look up documentation for a Rust crate (returns markdown)".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "crate_name": {
                            "type": "string",
                            "description": "The name of the crate to look up"
                        },
                        "version": {
                            "type": "string",
                            "description": "The version of the crate (optional, defaults to latest)"
                        }
                    },
                    "required": ["crate_name"]
                }),
            ),
            Tool::new(
                "search_crates".to_string(),
                "Search for Rust crates on crates.io (returns JSON or markdown)".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results to return (optional, defaults to 10, max 100)"
                        }
                    },
                    "required": ["query"]
                }),
            ),
            Tool::new(
                "lookup_item".to_string(),
                "Look up documentation for a specific item in a Rust crate (returns markdown)".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "crate_name": {
                            "type": "string",
                            "description": "The name of the crate"
                        },
                        "item_path": {
                            "type": "string",
                            "description": "Path to the item (e.g., 'vec::Vec' or 'crate_name::vec::Vec' - crate prefix will be automatically stripped)"
                        },
                        "version": {
                            "type": "string",
                            "description": "The version of the crate (optional, defaults to latest)"
                        }
                    },
                    "required": ["crate_name", "item_path"]
                }),
            ),
            Tool::new(
                "list_crate_items".to_string(),
                "Enumerate all items in a Rust crate (optionally filtered by type, visibility, or module). Returns a concise, categorized list.".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "crate_name": {
                            "type": "string",
                            "description": "The name of the crate"
                        },
                        "version": {
                            "type": "string",
                            "description": "The version of the crate"
                        },
                        "item_type": {
                            "type": "string",
                            "description": "Filter by item type (struct, enum, trait, fn, macro, mod)"
                        },
                        "visibility": {
                            "type": "string",
                            "description": "Filter by visibility (pub, private)"
                        },
                        "module": {
                            "type": "string",
                            "description": "Filter by module path (e.g., serde::de)"
                        }
                    },
                    "required": ["crate_name", "version"]
                }),
            ),
        ]
    }

    fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Content>, ToolError>> + Send + 'static>> {
        let this = self.clone();
        let tool_name = tool_name.to_string();
        let arguments = arguments.clone();

        Box::pin(async move {
            match tool_name.as_str() {
                "lookup_crate" => {
                    let crate_name = arguments
                        .get("crate_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("crate_name is required".to_string()))?
                        .to_string();
                    
                    let version = arguments
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    
                    let doc = this.lookup_crate(crate_name, version).await?;
                    Ok(vec![Content::text(doc)])
                }
                "search_crates" => {
                    let query = arguments
                        .get("query")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("query is required".to_string()))?
                        .to_string();
                    
                    let limit = arguments
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    
                    let results = this.search_crates(query, limit).await?;
                    Ok(vec![Content::text(results)])
                }
                "lookup_item" => {
                    let crate_name = arguments
                        .get("crate_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("crate_name is required".to_string()))?
                        .to_string();
                    
                    let item_path = arguments
                        .get("item_path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("item_path is required".to_string()))?
                        .to_string();
                    
                    let version = arguments
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    
                    let doc = this.lookup_item(crate_name, item_path, version).await?;
                    Ok(vec![Content::text(doc)])
                }
                "list_crate_items" => {
                    let crate_name = arguments
                        .get("crate_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("crate_name is required".to_string()))?
                        .to_string();
                    let version = arguments
                        .get("version")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ToolError::InvalidParameters("version is required".to_string()))?
                        .to_string();
                    let item_type = arguments
                        .get("item_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let visibility = arguments
                        .get("visibility")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let module = arguments
                        .get("module")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let filters = cratedocs_mcp::tools::item_list::ItemListFilters {
                        item_type,
                        visibility,
                        module,
                    };
                    let result = cratedocs_mcp::tools::item_list::list_crate_items(
                        &crate_name,
                        &version,
                        Some(filters),
                    )
                    .await
                    .map_err(|e| ToolError::ExecutionError(format!("list_crate_items failed: {}", e)))?;
                    Ok(vec![Content::text(result)])
                }
                _ => Err(ToolError::NotFound(format!("Tool {} not found", tool_name))),
            }
        })
    }

    fn list_resources(&self) -> Vec<Resource> {
        vec![]
    }

    fn read_resource(
        &self,
        _uri: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, ResourceError>> + Send + 'static>> {
        Box::pin(async move {
            Err(ResourceError::NotFound("Resource not found".to_string()))
        })
    }

    fn list_prompts(&self) -> Vec<Prompt> {
        vec![]
    }

    fn get_prompt(
        &self,
        prompt_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, PromptError>> + Send + 'static>> {
        let prompt_name = prompt_name.to_string();
        Box::pin(async move {
            Err(PromptError::NotFound(format!(
                "Prompt {} not found",
                prompt_name
            )))
        })
    }
}