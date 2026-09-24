# harper-mcp-server

Reference MCP (Model Context Protocol) server for Harper.

This crate is intentionally a small reference implementation. It demonstrates the JSON-RPC
surface Harper expects from an MCP endpoint: `initialize`, `tools/list`, and `tools/call`,
plus a simple rate limiter and health check. The bundled tools are limited to `echo` and
`get_time`.

It is not the production tool host. Harper's agent tools live in `harper-core`
(`lib/harper-core/src/tools`). The outbound MCP client path used by Harper is separate
and talks to a configured `mcp.server_url`.

## Run

```bash
cargo run -p harper-mcp-server
```

The server listens on `http://127.0.0.1:5001` and exposes `POST /` for JSON-RPC and
`GET /health` for liveness.

## Scope

- Protocol demonstration and integration smoke tests
- Stable place for MCP protocol experiments
- Not a full Harper tool surface, not a remote multi-tenant host

If you need Harper's real tools, use the Harper runtime rather than this binary.
