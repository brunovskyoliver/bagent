---
status: accepted
---

# Isolate agent sessions and share the browser profile

Each stdio MCP Agent Connection may own one private Browser Session, and several connections may run separate sessions concurrently. The connection is the enforceable owner because MCP tool calls do not reliably identify an individual conversation behind a shared client. Each session owns its WebKit view and independently visible popup. Other connections cannot inspect or control its live page, but all sessions use the same persistent Browser Profile so the user signs in only once. This deliberately trades strict authentication isolation for convenience. Logging out or changing account state in one session can affect other sessions on the same origin.
