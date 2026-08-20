# bagent

bagent is a private macOS assistant with agent-controlled tools and a notch-based user interface.

## Language

**bagent Browser**:
A bagent-owned web browser for agent-assisted development and manual review. It is separate from Safari.
_Avoid_: Safari browser, embedded Safari

**Browser Profile**:
Persistent website data owned only by bagent Browser and isolated from Safari.
_Avoid_: Safari profile, shared browser profile

**Browser Session**:
The live page and browser state assigned to one Agent Connection and retained by bagent after that connection ends. An Agent Connection may own at most one, while several connections may own separate sessions at the same time.
_Avoid_: MCP session, browser profile

**Agent Connection**:
One active stdio MCP proxy connection from an agent host to bagent Browser. It is the enforceable owner identity for a Browser Session.
_Avoid_: Conversation, model, agent name

**Detached Session**:
A live Browser Session whose owning Agent Connection has ended. It remains available to the user but inaccessible to agents until the user approves a new owner.
_Avoid_: Orphaned session, abandoned tab

**Navigation Allowlist**:
The browser destinations an agent may open without additional user approval.
_Avoid_: Trusted network, safe sites

**Browser Cue**:
The notch indicator showing that a Browser Session exists and whether it is idle, agent-controlled, or waiting for the user.
_Avoid_: Browser status, running dot

**Control Lease**:
The temporary right of a Browser Session's owning Agent Connection to change that session. Direct user input always revokes it.
_Avoid_: Browser lock, agent session

**Page Snapshot**:
A bounded semantic description of the current page, including visible content and interactive elements. It is not raw HTML or a screenshot.
_Avoid_: DOM dump, accessibility tree

**Element Reference**:
A short-lived opaque identifier for an interactive element in one Page Snapshot revision.
_Avoid_: CSS selector, XPath

**Submission Grant**:
User permission for one Agent Connection to submit forms on one origin during one Browser Session.
_Avoid_: Write access, permanent approval
