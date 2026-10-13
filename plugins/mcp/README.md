# zhang-mcp

An [MCP](https://modelcontextprotocol.io/) server for a zhang ledger, as one Lua script: `mcp.lua`. It gives an AI
assistant (Claude Code, Claude.ai, the MCP Inspector, any Streamable HTTP client) twelve read tools over the queries
the app runs, and, when you turn writes on, three tools that record an entry after showing you exactly what will be
written. It runs inside zhang's Lua plugin runtime: nothing to build, nothing to install.

## Install

1. Copy `mcp.lua` into your ledger, for example as `plugins/mcp.lua`. A release ships it as `zhang-mcp-<version>.lua`.
   (The next release bundles the script: `plugin "zhang:mcp"`, with no file to copy.)
2. Declare it:

   ```zhang
   option "features.plugin" "true"

   plugin "plugins/mcp.lua"
     timeout: "20s"
   ```

3. Reload. The endpoint is `POST /api/plugins/zhang-mcp/mcp`, behind zhang's own sign-in.

To let the assistant record entries, add the two switches and reload:

```zhang
plugin "plugins/mcp.lua"
  timeout: "20s"
  allow_writes: "true"
  allowed_writes: "transaction"
  allowed_writes: "open"
```

The guide, with the client setup, every tool and the write flow: https://zhang-accounting.kilerd.me/guides/mcp/
(the docs source is `docs/src/content/docs/guides/mcp.md`). The design, the specification rules and the deviations:
[`DESIGN.md`](./DESIGN.md). The end-to-end tests: `zhang-server/tests/mcp_plugin.rs`, which drives this file straight
from the repository.
