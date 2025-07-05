use anyhow::Result;
use clap::{Parser, Subcommand};
use cratedocs_mcp::tools::DocRouter;
use mcp_core::Content;
use mcp_server::router::RouterService;
use mcp_server::{ByteTransport, Router, Server};
use serde_json::json;
use std::net::SocketAddr;
use tokio::io::{stdin, stdout};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{self, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use regex::Regex;

#[derive(Parser)]
#[command(author, version = "0.2.0", about, long_about = None)]
#[command(propagate_version = true)]
#[command(disable_version_flag = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Output the version and exit
    Version,
    /// Run the server in stdin/stdout mode
    Stdio {
        /// Enable debug logging
        #[arg(short, long)]
        debug: bool,
    },
    /// Run the server with HTTP/SSE interface
    Http {
        /// Address to bind the HTTP server to
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        address: String,
        
        /// Enable debug logging
        #[arg(short, long)]
        debug: bool,
    },
    /// Test tools directly from the CLI
    Test {
        /// The tool to test (lookup_crate, search_crates, lookup_item, list_crate_items)
        #[arg(long, default_value = "lookup_crate")]
        tool: String,
        
        /// Crate name for lookup_crate, lookup_item, and list_crate_items
        #[arg(long)]
        crate_name: Option<String>,
        
        /// Item path for lookup_item (e.g., std::vec::Vec)
        #[arg(long)]
        item_path: Option<String>,
        
        /// Search query for search_crates
        #[arg(long)]
        query: Option<String>,
        
        /// Crate version (optional)
        #[arg(long)]
        version: Option<String>,
        
        /// Result limit for search_crates
        #[arg(long)]
        limit: Option<u32>,
        
        /// Filter by item type for list_crate_items (e.g., struct, enum, trait)
        #[arg(long)]
        item_type: Option<String>,
        
        /// Filter by visibility for list_crate_items (e.g., pub, private)
        #[arg(long)]
        visibility: Option<String>,
        
        /// Filter by module path for list_crate_items (e.g., serde::de)
        #[arg(long)]
        module: Option<String>,
        
        /// Output format (markdown, text, json)
        #[arg(long, default_value = "markdown")]
        format: Option<String>,
        
        /// Output file path (if not specified, results will be printed to stdout)
        #[arg(long)]
        output: Option<String>,
    
        /// Summarize output by stripping LICENSE and VERSION sections (TL;DR mode)
        #[arg(long)]
        tldr: bool,
    
        /// Maximum number of tokens for output (token-aware truncation)
        #[arg(long)]
        max_tokens: Option<usize>,
        
        /// Enable debug logging
        #[arg(short, long)]
        debug: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        },
        Commands::Stdio { debug } => run_stdio_server(debug).await,
        Commands::Http { address, debug } => run_http_server(address, debug).await,
        Commands::Test {
            tool,
            crate_name,
            item_path,
            query,
            version,
            limit,
            item_type,
            visibility,
            module,
            format,
            output,
            tldr,
            max_tokens,
            debug
        } => run_test_tool(TestToolConfig {
            tool,
            crate_name,
            item_path,
            query,
            version,
            limit,
            item_type,
            visibility,
            module,
            format,
            output,
            tldr,
            max_tokens,
            debug
        }).await,
    }
}

async fn run_stdio_server(debug: bool) -> Result<()> {
    // Set up file appender for logging
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "stdio-server.log");

    // Initialize the tracing subscriber with file logging
    let level = if debug { tracing::Level::DEBUG } else { tracing::Level::INFO };
    
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(level.into()))
        .with_writer(file_appender)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("Starting MCP documentation server in STDIN/STDOUT mode");

    // Create an instance of our documentation router
    let router = RouterService(DocRouter::new());

    // Create and run the server
    let server = Server::new(router);
    let transport = ByteTransport::new(stdin(), stdout());

    tracing::info!("Documentation server initialized and ready to handle requests");
    Ok(server.run(transport).await?)
}

async fn run_http_server(address: String, debug: bool) -> Result<()> {
    // Setup tracing
    let level = if debug { "debug" } else { "info" };
    
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{},{}", level, env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse socket address
    let addr: SocketAddr = address.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::debug!("Rust Documentation Server listening on {}", listener.local_addr()?);
    tracing::info!("Access the Rust Documentation Server at http://{}/sse", addr);
    
    // Create app and run server
    let app = cratedocs_mcp::transport::http_sse_server::App::new();
    axum::serve(listener, app.router()).await?;
    
    Ok(())
}

// --- TLDR Helper Function ---
fn apply_tldr(input: &str) -> String {
    // Remove LICENSE and VERSION(S) sections by skipping lines between those headings and the next heading or EOF.
    let mut output = Vec::new();
    let mut skip = false;

    // Match any heading (with or without space) for LICENSE or VERSION(S)
    let tldr_section_re = Regex::new(r"(?i)^\s*#+\s*license\b|^\s*#+\s*version(s)?\b|^\s*#+license\b|^\s*#+version(s)?\b").unwrap();
    // Match any heading (for ending the skip)
    let heading_re = Regex::new(r"^\s*#+").unwrap();

    for line in input.lines() {
        // Start skipping if we hit a LICENSE or VERSION(S) heading
        if !skip && tldr_section_re.is_match(line) {
            skip = true;
            continue; // skip the heading line itself
        }
        // Stop skipping at the next heading (but do not skip the heading itself)
        if skip && heading_re.is_match(line) && !tldr_section_re.is_match(line) {
            skip = false;
        }
        if !skip {
            output.push(line);
        }
    }
    output.join("\n")
}

/// Configuration for the test tool
struct TestToolConfig {
    tool: String,
    crate_name: Option<String>,
    item_path: Option<String>,
    query: Option<String>,
    version: Option<String>,
    limit: Option<u32>,
    item_type: Option<String>,
    visibility: Option<String>,
    module: Option<String>,
    format: Option<String>,
    output: Option<String>,
    tldr: bool,
    max_tokens: Option<usize>,
    debug: bool,
}

/// Run a direct test of a documentation tool from the CLI
async fn run_test_tool(config: TestToolConfig) -> Result<()> {
    let TestToolConfig {
        tool,
        crate_name,
        item_path,
        query,
        version,
        limit,
        format,
        output,
        tldr,
        max_tokens,
        debug,
        item_type,
        visibility,
        module,
    } = config;
    // Print help information if the tool is "help"
    if tool == "help" {
        println!("CrateDocs CLI Tool Tester\n");
        println!("Usage examples:");
        println!("  cargo run --bin cratedocs -- test --tool lookup_crate --crate-name serde");
        println!("  cargo run --bin cratedocs -- test --tool lookup_crate --crate-name tokio --version 1.35.0");
        println!("  cargo run --bin cratedocs -- test --tool lookup_item --crate-name tokio --item-path sync::mpsc::Sender");
        println!("  cargo run --bin cratedocs -- test --tool lookup_item --crate-name serde --item-path Serialize --version 1.0.147");
        println!("  cargo run --bin cratedocs -- test --tool search_crates --query logger --limit 5");
        println!("  cargo run --bin cratedocs -- test --tool search_crates --query logger --format json");
        println!("  cargo run --bin cratedocs -- test --tool lookup_crate --crate-name tokio --output tokio-docs.md");
        println!("\nAvailable tools:");
        println!("  lookup_crate   - Look up documentation for a Rust crate");
        println!("  lookup_item    - Look up documentation for a specific item in a crate");
        println!("                   Format: 'module::path::ItemName' (e.g., 'sync::mpsc::Sender')");
        println!("                   The tool will try to detect if it's a struct, enum, trait, fn, or macro");
        println!("  search_crates  - Search for crates on crates.io");
        println!("  help           - Show this help information");
        println!("\nOutput options:");
        println!("  --format       - Output format: markdown (default), text, json");
        println!("  --output       - Write output to a file instead of stdout");
        println!("  --tldr         - Summarize output by stripping LICENSE and VERSION sections");
        return Ok(());
    }
    
    // Set up console logging
    let level = if debug { tracing::Level::DEBUG } else { tracing::Level::INFO };
    
    tracing_subscriber::fmt()
        .with_max_level(level)
        .without_time()
        .with_target(false)
        .init();

    // Create router instance
    let router = DocRouter::new();
    
    tracing::info!("Testing tool: {}", tool);
    
    // Get format option (default to markdown)
    let format = format.unwrap_or_else(|| "markdown".to_string());
    
    // Prepare arguments based on the tool being tested
    let arguments = match tool.as_str() {
        "lookup_crate" => {
            let crate_name = crate_name.ok_or_else(|| 
                anyhow::anyhow!("--crate-name is required for lookup_crate tool"))?;
            
            json!({
                "crate_name": crate_name,
                "version": version,
            })
        },
        "lookup_item" => {
            let crate_name = crate_name.ok_or_else(|| 
                anyhow::anyhow!("--crate-name is required for lookup_item tool"))?;
            let item_path = item_path.ok_or_else(|| 
                anyhow::anyhow!("--item-path is required for lookup_item tool"))?;
            
            json!({
                "crate_name": crate_name,
                "item_path": item_path,
                "version": version,
            })
        },
        "search_crates" => {
            let query = query.ok_or_else(|| 
                anyhow::anyhow!("--query is required for search_crates tool"))?;
            
            json!({
                "query": query,
                "limit": limit,
            })
        },
        "list_crate_items" => {
            let crate_name = crate_name.ok_or_else(||
                anyhow::anyhow!("--crate-name is required for list_crate_items tool"))?;
            let version = version.ok_or_else(||
                anyhow::anyhow!("--version is required for list_crate_items tool"))?;
            
            let arguments = json!({
                "crate_name": crate_name,
                "version": version,
                "item_type": item_type,
                "visibility": visibility,
                "module": module,
            });
            arguments
        },
        _ => return Err(anyhow::anyhow!("Unknown tool: {}", tool)),
    };
    
    // Call the tool and get results
    tracing::debug!("Calling {} with arguments: {}", tool, arguments);
    println!("Executing {} tool...", tool);
    
    let result = match router.call_tool(&tool, arguments).await {
        Ok(result) => result,
        Err(e) => {
            eprintln!("\nERROR: {}", e);
            eprintln!("\nTip: Try these suggestions:");
            eprintln!("  - For crate docs: cargo run --bin cratedocs -- test --tool lookup_crate --crate-name tokio");
            eprintln!("  - For item lookup: cargo run --bin cratedocs -- test --tool lookup_item --crate-name tokio --item-path sync::mpsc::Sender");
            eprintln!("  - For item lookup with version: cargo run --bin cratedocs -- test --tool lookup_item --crate-name serde --item-path Serialize --version 1.0.147");
            eprintln!("  - For crate search: cargo run --bin cratedocs -- test --tool search_crates --query logger --limit 5");
            eprintln!("  - For output format: cargo run --bin cratedocs -- test --tool search_crates --query logger --format json");
            eprintln!("  - For file output: cargo run --bin cratedocs -- test --tool lookup_crate --crate-name tokio --output tokio-docs.md");
            eprintln!("  - For help: cargo run --bin cratedocs -- test --tool help");
            return Ok(());
        }
    };
    
    // Process and output results
    if !result.is_empty() {
        for content in result {
            if let Content::Text(text) = content {
                let mut content_str = text.text;

                // If max_tokens is set, truncate output to fit within the limit
                if let Some(max_tokens) = max_tokens {
                    match cratedocs_mcp::tools::count_tokens(&content_str) {
                        Ok(token_count) if token_count > max_tokens => {
                            // Truncate by character, then to previous word boundary, and append Mandarin to indicate truncation.
                            let mut truncated = content_str.clone();
                            while cratedocs_mcp::tools::count_tokens(&truncated).map_or(0, |c| c) > max_tokens && !truncated.is_empty() {
                                truncated.pop();
                            }
                            if let Some(last_space) = truncated.rfind(' ') {
                                truncated.truncate(last_space);
                            }
                            truncated.push_str(" 内容被截断");
                            content_str = truncated;
                        }
                        _ => {}
                    }
                }

                // TL;DR processing: strip LICENSE and VERSION(S) sections if --tldr is set
                if tldr {
                    content_str = apply_tldr(&content_str);
                }

                let formatted_output = match format.as_str() {
                    "json" => {
                        // For search_crates, which may return JSON content
                        if tool == "search_crates" && content_str.trim().starts_with('{') {
                            // If content is already valid JSON, pretty print it
                            match serde_json::from_str::<serde_json::Value>(&content_str) {
                                Ok(json_value) => serde_json::to_string_pretty(&json_value)
                                    .unwrap_or_else(|_| content_str.clone()),
                                Err(_) => {
                                    // If it's not JSON, wrap it in a simple JSON object
                                    json!({ "content": content_str }).to_string()
                                }
                            }
                        } else {
                            // For non-JSON content, wrap in a JSON object
                            json!({ "content": content_str }).to_string()
                        }
                    },
                    "text" => {
                        // For JSON content, try to extract plain text
                        if content_str.trim().starts_with('{') && tool == "search_crates" {
                            match serde_json::from_str::<serde_json::Value>(&content_str) {
                                Ok(json_value) => {
                                    // Try to create a simple text representation of search results
                                    if let Some(crates) = json_value.get("crates").and_then(|v| v.as_array()) {
                                        let mut text_output = String::from("Search Results:\n\n");
                                        for (i, crate_info) in crates.iter().enumerate() {
                                            let name = crate_info.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown");
                                            let description = crate_info.get("description").and_then(|v| v.as_str()).unwrap_or("No description");
                                            let downloads = crate_info.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                                            
                                            text_output.push_str(&format!("{}. {} - {} (Downloads: {})\n",
                                                i + 1, name, description, downloads));
                                        }
                                        text_output
                                    } else {
                                        content_str
                                    }
                                },
                                Err(_) => content_str,
                            }
                        } else {
                            // For markdown content, use a simple approach to convert to plain text
                            // This is a very basic conversion - more sophisticated would need a proper markdown parser
                            content_str
                                .replace("# ", "")
                                .replace("## ", "")
                                .replace("### ", "")
                                .replace("#### ", "")
                                .replace("##### ", "")
                                .replace("###### ", "")
                                .replace("**", "")
                                .replace("*", "")
                                .replace("`", "")
                        }
                    },
                    _ => content_str, // Default to original markdown for "markdown" or any other format
                };
                
                // Output to file or stdout
                match &output {
                    Some(file_path) => {
                        use std::fs;
                        use std::io::Write;
                        
                        tracing::info!("Writing output to file: {}", file_path);
                        
                        // Ensure parent directory exists
                        if let Some(parent) = std::path::Path::new(file_path).parent() {
                            if !parent.exists() {
                                fs::create_dir_all(parent)?;
                            }
                        }
                        
                        let mut file = fs::File::create(file_path)?;
                        file.write_all(formatted_output.as_bytes())?;
                        println!("Results written to file: {}", file_path);
                    },
                    None => {
                        // Print to stdout
                        println!("\n--- TOOL RESULT ---\n");
                        println!("{}", formatted_output);
                        println!("\n--- END RESULT ---");
                    }
                }
            } else {
                println!("Received non-text content");
            }
        }
    } else {
        println!("Tool returned no results");
    }
    
    Ok(())
}
#[cfg(test)]
mod tldr_tests {
    use super::apply_tldr;

    #[test]
    fn test_apply_tldr_removes_license_and_versions() {
        let input = r#"
# Versions
This is version info.

# LICENSE
MIT License text.

# Usage
Some real documentation here.

# Another Section
More docs.
"#;
        let output = apply_tldr(input);
        assert!(!output.to_lowercase().contains("license"));
        assert!(!output.to_lowercase().contains("version"));
        assert!(output.contains("Usage"));
        assert!(output.contains("Another Section"));
        assert!(output.contains("Some real documentation here."));
        // Debug print for failure analysis
        if output.to_lowercase().contains("license") {
            println!("DEBUG OUTPUT:\n{}", output);
        }
    }

    #[test]
    fn test_apply_tldr_handles_no_license_or_versions() {
        let input = r#"
# Usage
Some real documentation here.
"#;
        let output = apply_tldr(input);
        assert_eq!(output.trim(), input.trim());
    }
#[test]
fn test_apply_tldr_no_headings() {
    let input = r#"
This is plain text without any headings.
It should remain unchanged after processing.
"#;
    let output = apply_tldr(input);
    assert_eq!(output.trim(), input.trim());
}

#[test]
fn test_apply_tldr_malformed_markdown() {
    let input = r#"
#LICENSE
This is a malformed license heading.
#VERSION
This is a malformed version heading.
"#;
    let output = apply_tldr(input);
    assert!(!output.to_lowercase().contains("license"));
    assert!(!output.to_lowercase().contains("version"));
}

#[test]
fn test_apply_tldr_large_input() {
    let input = r#"
# Versions
Version 1.0.0
Version 2.0.0

# LICENSE
MIT License text.

# Usage
Some real documentation here.

# Another Section
More docs.

# LICENSE
Another license section.

# Versions
Another version section.
"#;
    let output = apply_tldr(input);
    assert!(!output.to_lowercase().contains("license"));
    assert!(!output.to_lowercase().contains("version"));
    assert!(output.contains("Usage"));
    assert!(output.contains("Another Section"));
    assert!(output.contains("Some real documentation here."));
}
}
