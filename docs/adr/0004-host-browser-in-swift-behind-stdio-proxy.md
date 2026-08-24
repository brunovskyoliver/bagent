---
status: accepted
---

# Host the browser in Swift behind a stdio MCP proxy

The macOS app owns all WebKit views, Browser Sessions, panels, and policy on the main actor. A bundled Rust executable implements MCP over stdio and forwards commands to the running app through a user-only Unix socket. This keeps browser state in the only process that can safely own AppKit and WebKit while giving Codex and Claude a conventional local MCP server. The proxy is stateless and never starts an independent browser instance.
