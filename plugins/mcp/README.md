# zhang-mcp

An [MCP](https://modelcontextprotocol.io/) server for a zhang ledger, as one Lua script: `mcp.lua`. It gives an AI
assistant (Claude Code, Claude.ai, the MCP Inspector, any Streamable HTTP client) twelve read tools over the queries
the app runs, and, when you turn writes on, three tools that record an entry after showing you exactly what will be
written. It runs inside zhang's Lua plugin runtime: nothing to build, nothing to install.

## Install

zhang bundles this script: a ledger declares it by the name `zhang:mcp`, with no file to copy, wherever the ledger is
stored (a local directory, S3 or R2, WebDAV, GitHub).

```zhang
option "features.plugin" "true"

plugin "zhang:mcp"
  timeout: "20s"
```

Reload. The endpoint is `POST /api/plugins/zhang-mcp/mcp`, behind zhang's own sign-in.

To let the assistant record entries, add the two switches and reload:

```zhang
plugin "zhang:mcp"
  timeout: "20s"
  allow_writes: "true"
  allowed_writes: "transaction"
  allowed_writes: "open"
```

The bundled copy is this file as it was when your zhang was built (`include_bytes!` in
`zhang-core/src/plugin/bundled.rs`), so updating zhang updates it, and it is granted only what its directive grants.
To customise the plugin, copy `mcp.lua` into your ledger instead (a release ships it as `zhang-mcp-<version>.lua`),
edit it, and declare it by its path, `plugin "plugins/mcp.lua"`, with the same metadata lines.

The guide, with the client setup, every tool and the write flow: https://zhang-accounting.kilerd.me/guides/mcp/
(the docs source is `docs/src/content/docs/guides/mcp.md`). The design, the specification rules and the deviations:
[`DESIGN.md`](./DESIGN.md). The end-to-end tests: `zhang-server/tests/mcp_plugin.rs`, which drives this file straight
from the repository.
