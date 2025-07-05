# CrateDocs MCP

This is an MCP (Model Context Protocol) server that provides tools for Rust crate documentation lookup. It allows LLMs to look up documentation for Rust crates they are unfamiliar with.

## Features

- Lookup crate documentation: Get general documentation for a Rust crate
- Search crates: Search for crates on crates.io based on keywords
- Lookup item documentation: Get documentation for a specific item (e.g., struct, function, trait) within a crate

## Installation

```bash
git clone https://github.com/d6e/cratedocs-mcp.git
cd cratedocs-mcp
cargo build --release
```

## Running the Server

There are multiple ways to run the documentation server:

### Using the Unified CLI

The unified command-line interface provides subcommands for all server modes:

```bash
# Run in STDIN/STDOUT mode
cargo run --bin cratedocs stdio

# Run in HTTP/SSE mode (default address: 127.0.0.1:8080)
cargo run --bin cratedocs http

# Run in HTTP/SSE mode with custom address
cargo run --bin cratedocs http --address 0.0.0.0:3000

# Enable debug logging
cargo run --bin cratedocs http --debug
```

### Directly Testing Documentation Tools

You can directly test the documentation tools from the command line without starting a server:

```bash
# Get help for the test command
cargo run --bin cratedocs test --tool help

# Look up crate documentation
cargo run --bin cratedocs test --tool lookup_crate --crate-name tokio

# Look up item documentation
cargo run --bin cratedocs test --tool lookup_item --crate-name tokio --item-path sync::mpsc::Sender

# Look up documentation for a specific version
cargo run --bin cratedocs test --tool lookup_item --crate-name serde --item-path Serialize --version 1.0.147

# Look up a trait in a crate (e.g., the Serialize trait in serde) & a specific version
cargo run --bin cratedocs test --tool lookup_item --crate-name serde --item-path serde::Serialize --version 1.0.160

# Search for crates
cargo run --bin cratedocs test --tool search_crates --query logger --limit 5

# Output in different formats (markdown, text, json)
cargo run --bin cratedocs test --tool search_crates --query logger --format json
cargo run --bin cratedocs test --tool lookup_crate --crate-name tokio --format text

# Save output to a file
cargo run --bin cratedocs test --tool lookup_crate --crate-name tokio --output tokio-docs.md

# Summarize output by stripping LICENSE and VERSION sections, limits to xxxxx tokens (uses huggingface tokenizer)
cargo run --bin cratedocs test --tool lookup_crate --crate-name tokio --tldr --max_tokens 48000
```

By default, the HTTP server will listen on `http://127.0.0.1:8080/sse`.

## Available Tools

The server provides the following tools:

### 1. `lookup_crate`

Retrieves documentation for a specified Rust crate.

Parameters:
- `crate_name` (required): The name of the crate to look up
- `version` (optional): The version of the crate (defaults to latest)

Example:
```json
{
  "name": "lookup_crate",
  "arguments": {
    "crate_name": "tokio",
    "version": "1.28.0"
  }
}
```

### 2. `search_crates`

Searches for Rust crates on crates.io.

Parameters:
- `query` (required): The search query
- `limit` (optional): Maximum number of results to return (defaults to 10, max 100)

Example:
```json
{
  "name": "search_crates",
  "arguments": {
    "query": "async runtime",
    "limit": 5
  }
}
```

### 3. `lookup_item`

Retrieves documentation for a specific item in a crate.

Parameters:
- `crate_name` (required): The name of the crate
- `item_path` (required): Path to the item (e.g., 'std::vec::Vec')
- `version` (optional): The version of the crate (defaults to latest)

Example:
```json
{
  "name": "lookup_item",
  "arguments": {
    "crate_name": "serde",
    "item_path": "serde::Deserialize",
    "version": "1.0.160"
  }
}
```

## Implementation Notes

- The server includes a caching mechanism to prevent redundant API calls for the same documentation
- It interfaces with docs.rs for crate documentation and crates.io for search functionality
- Results are returned as plain text/HTML content that can be parsed and presented by the client

## MCP Protocol Integration

This server implements the Model Context Protocol (MCP) which allows it to be easily integrated with LLM clients that support the protocol. For more information about MCP, visit [the MCP repository](https://github.com/modelcontextprotocol/mcp).

## License

MIT License

## MCP Tool: `list_crate_items`

The `list_crate_items` tool enumerates all items in a specified Rust crate and version, optionally filtering by item type, visibility, or module path. This is useful for quickly exploring the structure of a crate, generating concise listings for LLMs, or programmatically analyzing crate APIs.

### Usage

```sh
cargo run --bin cratedocs -- list-crate-items --crate-name serde --version 1.0.0
```

#### With filters:

- Filter by item type (e.g., struct, enum, trait, fn, macro, mod):

  ```sh
  cargo run --bin cratedocs -- list-crate-items --crate-name serde --version 1.0.0 --item-type struct
  ```

- Filter by visibility (e.g., pub, private):

  ```sh
  cargo run --bin cratedocs -- list-crate-items --crate-name serde --version 1.0.0 --visibility pub
  ```

- Filter by module path:

  ```sh
  cargo run --bin cratedocs -- list-crate-items --crate-name serde --version 1.0.0 --module serde::de
  ```

### Output

The output is a concise, categorized list (JSON or markdown) showing each item's name, type, visibility, and module path.

**Example (stub output):**
```
Stub: list_crate_items for crate: serde, version: 1.0.0, filters: Some(ItemListFilters { item_type: Some("struct"), visibility: None, module: None })
```

When implemented, the output will be a structured list of items matching the filters.
