install:
   cargo install --git https://github.com/PromptExecution/cratedocs-mcp --locked

run:
   cargo run --bin cratedocs http --address 0.0.0.0:3000 --debug

debug-mcp-remote:
   # use bunx or npx to see how the mcp-remote proxy connects
   bunx mcp-remote@latest "http://127.0.0.1:3000/sse" --allow-http --transport sse-only --debug

