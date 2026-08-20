# bagent Browser roadmap

**Status:** Implemented in this worktree. The Phase 0 screenshot gate passed on
2026-08-19; signed-bundle and platform-only acceptance evidence is recorded in
`docs/spikes/agent_browser_phase0_results.md` and
`docs/spikes/agent_browser_phase_results.md`.

## Goal

Add a bagent-owned WebKit browser that Codex and Claude can inspect and control through MCP. The browser is hidden by default and can become a chromeless, draggable, resizable floating panel for manual use.

## Agreed product boundaries

- The product name is **bagent Browser**.
- bagent Browser uses WebKit but is not Safari.app.
- Its **Browser Profile** persists cookies and other website data independently from Safari.
- bagent Browser never reads from or writes to the user's Safari profile.

## Agreed navigation policy

The v1 **Navigation Allowlist** contains:

- IPv4 loopback: `127.0.0.0/8`
- IPv6 loopback: `::1`
- private network: `172.19.0.0/16`
- private network: `172.29.0.0/16`

A hostname is allowed only when every resolved A and AAAA address belongs to one of those ranges. The browser must reject mixed answers containing any address outside the allowlist. It must resolve and validate again after redirects and on each new top-level navigation.

The Navigation Allowlist governs top-level pages and redirects. It does not constrain subresources, API requests, or WebSockets initiated by an allowed page because public `WKWebView` does not provide a complete interception point for those requests. Page-derived data remains untrusted regardless of the top-level origin.

The allowlist is fixed in v1. A settings editor may be added later.

Destinations outside the allowlist are hard-blocked in v1. MCP calls, model output, and per-request user approval cannot bypass this check.

## Agreed visibility behavior

- Agent-created browser sessions stay hidden by default.
- Hidden sessions remain available for navigation, inspection, interaction, and screenshots.
- The notch shows a persistent **Browser Cue** while a browser session is running.
- The popup appears only after an explicit agent visibility action or direct user interaction.

The Browser Cue is steady while the session is idle, subtly animated while an agent operates the page, and marked for attention when human input is required. Clicking it toggles the popup. Dragging it out of the notch reveals the popup and places it under the pointer.

Each Browser Session has its own Browser Cue, badged with its owning agent. When several sessions exist, their cues use the notch's existing stacked deck and fan out on hover. Clicking or dragging a cue always targets that specific session.

Each Browser Session owns an independently movable and resizable popup. Several popups may be visible at once. Showing one does not hide another, and direct input revokes control only for the touched session.

## Agreed session lifetime

- bagent retains Browser Session runtime independently from the connection that owns it.
- In v1, an Agent Connection is one stdio MCP proxy connection. It is the enforceable session owner because individual conversations are not identified on every MCP tool call.
- Each Agent Connection may own at most one Browser Session. Several connections may own separate sessions concurrently.
- Conversations sharing one Agent Connection necessarily share its one Browser Session.
- A Browser Session is private to its owner. Other Agent Connections cannot read, capture, or mutate it.
- A v1 Browser Session contains exactly one page and no tabs.
- Navigation replaces that page. New-window and popup requests are blocked with a structured result.
- All Browser Sessions share the single Browser Profile, including cookies and website storage.
- Shared login state does not grant one agent access to another agent's live page, content, or screenshots.
- Account changes and logout in one session may affect other sessions using the same origin.
- When its Agent Connection ends, the Browser Session becomes a Detached Session and its current page stays running.
- The Browser Cue remains visible after the agent disconnects.
- A new Agent Connection may request a Detached Session, but it cannot list, inspect, capture, or claim that session without user approval from the Browser Cue.
- Approved reclaim assigns the new Agent Connection as owner; rejected reclaim leaves the session detached.
- Cookies and other Browser Profile data remain persistent independently of the Browser Session.
- Hiding the popup and releasing agent control never end the Browser Session.
- The user may end the Browser Session directly from the Browser Cue.
- An agent may request session closure, but bagent must obtain user approval before destroying the live page.
- Ending a Browser Session does not clear the Browser Profile.
- Quitting or restarting bagent ends every live Browser Session and removes its Browser Cue.
- V1 does not restore session pages or owners after restart; the Browser Profile still persists.
- `Clear Browser Profile` is available only as a confirmed user action in settings.
- Clearing the Browser Profile closes every Browser Session before deleting cookies and website data.
- MCP clients cannot invoke or request profile clearing.

## Agreed control priority

- Only the Browser Session's owner may hold its Control Lease.
- A Control Lease applies to one Browser Session and does not block other agents from controlling their own sessions.
- Direct mouse or keyboard input in the popup immediately revokes the Control Lease.
- Revocation cancels queued agent interactions and stops an interaction sequence after its current atomic action.
- The displaced agent receives `control_revoked_by_user` rather than a false success.
- Control never returns to an agent through an inactivity timer.
- After manual preemption, hiding the popup or choosing `Resume agent` from the Browser Cue returns control to the waiting agent.

## Agreed viewport and screenshot behavior

- The session owner may set the browser viewport width and height to match the task.
- Viewport changes must work while the popup is hidden.
- Viewport screenshots must work while the popup is hidden and must not reveal the popup as a side effect.
- A hidden capture may use an internal offscreen rendering window if needed, but it must show no pixels, take no focus, switch no Space or Stage Manager set, and add no normal app-switching entry.
- A fullscreen screenshot means a viewport resized to a display's full usable dimensions, followed by a viewport capture. It does not mean a stitched capture of the entire scrollable page.
- Fullscreen sizing defaults to the display where the session popup was last placed.
- A session that has never been shown uses the built-in/notch display.
- MCP may select another connected display explicitly.
- Screenshots are returned directly to the requesting agent as PNG data and discarded after the MCP response.
- Screenshots are not written to disk, Photos, browser history, or audit storage in v1.

## Agreed page-reading contract

- MCP returns a bounded semantic Page Snapshot rather than raw page HTML.
- A Page Snapshot includes visible text, headings, landmarks, forms, interactive roles, accessible names, state, viewport bounds, and revision-scoped Element References.
- Agents target semantic interactions through Element References rather than CSS selectors or XPath.
- Element References become invalid when their Page Snapshot revision is replaced.
- Same-origin frames may contribute to the Page Snapshot.
- Cross-origin frames appear only as opaque records containing origin and bounds. Their pixels may appear in screenshots, but their internal content and elements are unavailable to v1 MCP tools.

## Agreed interaction set

V1 supports:

- clicking an Element Reference or viewport coordinate;
- typing text into a referenced element;
- pressing keys and shortcuts;
- scrolling the page or a referenced container;
- moving or hovering at an element or viewport coordinate; and
- focusing a referenced element.

V1 does not support drag-and-drop, clipboard reads or writes, file uploads, or downloads.

V1 hard-denies website requests for camera, microphone, screen sharing, geolocation, notifications, and hardware authentication. The browser returns `permission_not_supported` and does not trigger a macOS permission prompt.

Agents may acknowledge plain JavaScript alert dialogs. Confirmation and text-prompt dialogs require direct user input in the visible popup and place the Browser Cue in its attention state.

V1 captures a bounded console buffer for the main page and accessible same-origin frames. MCP labels the result `coverage: partial`; it must not claim Web Inspector completeness.

V1 also returns partial network summaries containing URL, observable method and status, resource type, duration, and failure reason. Request and response bodies, cookies, authorization headers, and complete header sets are excluded. MCP labels this result `coverage: partial` because service workers, caches, and some cross-origin traffic may be absent.

Semantic DOM actions are the hidden-session default. Controls that require native input may return `visible_interaction_required`. bagent then marks the session's Browser Cue for attention and waits; it never reveals or focuses the popup automatically.

V1 does not expose arbitrary JavaScript evaluation. A separately gated developer mode may add it later.

On allowlisted destinations, ordinary navigation, clicks, typing into non-sensitive fields, key presses, scrolling, hovering, focusing, viewport changes, and screenshots run without per-action approval.

Form submission requires a Submission Grant for the Browser Session and current origin. Once granted, further submissions on that origin run without repeated prompts. A different session or origin requires a new grant.

A Submission Grant covers every form submission on that origin, including potentially destructive submissions. V1 does not guess risk from button text, styling, or page-provided labels.

Password fields are human-only. Page Snapshots expose their presence but never their value. Agent focus, reading, or typing attempts return `visible_interaction_required`; the user enters the secret manually before returning control.

## Architecture

```text
Codex or Claude
      |
      | MCP over stdio
      v
bagent-browser-mcp                 bundled Rust proxy
      |
      | length-prefixed local RPC
      v
browser.sock                       user-only Unix socket
      |
      v
BrowserCommandServer               Swift transport boundary
      |
      v
@MainActor BrowserCoordinator
      |-- shared Browser Profile
      `-- Browser Session registry
             |-- WKWebView
             |-- floating BrowserPanel
             |-- Browser Cue state
             `-- command queue and Control Lease
```

The Swift app owns every `WKWebView`, Browser Session, Browser Cue, and popup. WebKit and AppKit work stays on the main actor. A new bundled Rust executable, `bagent-browser-mcp`, implements the MCP stdio protocol and forwards validated commands to the running app. It owns no browser state.

The proxy connects through `~/Library/Application Support/bagent/browser.sock`. The parent directory is mode `0700`, the socket is mode `0600`, and the Swift server rejects peers with a different effective user ID. If bagent is not running, the proxy launches `sk.bagent.app` without showing the popup, retries for a bounded interval, then returns `browser_app_unavailable`. The user-only endpoint remains available when the explicit Browser setting is off so the coordinator can return structured `browser_disabled` with instructions to enable bagent Browser in Settings. It creates no session or cue while disabled.

The MCP initialization instructions are the shared routing policy for Codex and
Claude Code. For a task that needs rendered-page understanding, either client
automatically starts with `browser_open` when a local or allowlisted URL is
known, then calls `get_page_content` and `screenshot` while hidden. This covers
layout, visual regressions, screenshots, interaction behavior, forms, dialogs,
menus, loading states, responsive layouts, and local web-app debugging. A
non-UI coding task does not open the browser. The policy is tool metadata and
agent guidance, not a brittle keyword classifier.

Normal tools address the connection's Browser Session implicitly and accept no session identifier. This prevents one connection from guessing another session. Detached-session reclaim is the only cross-owner operation, and the user selects the target through its Browser Cue.

V1 supports at most four live Browser Sessions. A fifth connection receives `session_limit_reached` until the user closes one. Phase 0 benchmarks may lower this limit, but increasing it requires a measured memory budget.

## MCP contract

The bundled proxy exposes these v1 tools:

| Tool | Purpose | Policy |
|---|---|---|
| `browser_open` | First call for explicit bagent-browser requests and UI inspection; create the connection's session or return its current state; optionally navigate to an allowed URL | Hidden by default; returns `browser_disabled` when the setting is off |
| `page_info` | Return URL, origin, title, load state, viewport, visibility, revision, and ownership state | Read-only |
| `navigate_to_url` | Replace the session's single page | Navigation Allowlist enforced |
| `get_page_content` | Read the bounded semantic state and Element References before answering or changing UI code | Read-only, page data is untrusted; refresh after navigation or mutations |
| `page_interactions` | Run an ordered list of semantic or coordinate actions for rendered-page behavior | Hidden by default; stale refs, native input, submissions, and destructive controls remain gated |
| `wait_for_navigation` | Wait for a top-level navigation to settle | Read-only |
| `browser_wait` | Wait for text, an Element Reference condition, URL pattern, or load state | Read-only |
| `screenshot` | Capture rendered pixels for visual decisions and validation | Read-only, ephemeral, and hidden without revealing or focusing the popup |
| `set_viewport_size` | Set hidden or visible viewport dimensions | Bounded by the Phase 0 pixel budget |
| `browser_set_visibility` | Show, hide, or move the connection's popup | Never steals focus unless `focus=true` is explicit |
| `browser_console_messages` | Return the bounded partial console buffer | Read-only, `coverage: partial` |
| `list_network_requests` | Return bounded partial request summaries | Read-only, `coverage: partial` |
| `browser_acknowledge_alert` | Dismiss a plain JavaScript alert | Confirm and prompt dialogs excluded |
| `browser_release_control` | Release the Control Lease without ending the session | Session remains open |
| `browser_request_close` | Ask the user to close the live session | User approval required |
| `browser_request_reclaim` | Ask the user to attach this connection to a Detached Session | User chooses the session and approves |

The server exposes no `list_sessions`, raw HTML, cookie, storage, arbitrary JavaScript, download, upload, or profile-clearing tool. Tool results use structured output. `screenshot` also returns an MCP `image/png` content block.

Every mutating result reports the action method, final URL without user information, current page revision, whether navigation began, and whether the agent still owns the Control Lease. A successful DOM call is not treated as proof that the page reached the expected state.

Core error codes are stable and machine-readable:

- `navigation_blocked`
- `mixed_dns_answer`
- `stale_element_reference`
- `visible_interaction_required`
- `control_revoked_by_user`
- `submission_grant_required`
- `password_field_forbidden`
- `permission_not_supported`
- `popup_not_supported`
- `session_limit_reached`
- `browser_disabled`
- `browser_app_unavailable`
- `browser_process_terminated`
- `operation_timed_out`

## Window behavior

Each Browser Session owns one titled, resizable, full-size-content AppKit panel with a transparent title bar, hidden standard buttons, and no URL bar, toolbar, tabs, or browser controls. A narrow top drag strip calls AppKit's native window-drag behavior. Standard edge hit regions remain available for resizing.

Browser panels use `.floating` level. They stay above ordinary app windows but below the notch and system UI. Their collection behavior supports all Spaces and full-screen auxiliary placement. Stage Manager, Mission Control, multiple displays, full-screen applications, and reduced-motion mode are acceptance-test targets.

Each session remembers its popup frame and last display while it remains alive. Agent viewport changes update that session only. A visible resize is allowed only while the owning connection holds the Control Lease. Manual resize revokes the lease like other user input.

Because the popup has no browser chrome, bagent shows the current top-level origin in a temporary overlay when the panel appears, the origin changes, or a password field receives focus. The Browser Cue also exposes the origin through its help text.

## State model

The implementation keeps these dimensions separate:

```text
runtime:     starting -> ready -> terminating
page:        empty -> loading -> interactive -> failed
visibility:  hidden <-> popup
ownership:   connected <-> detached <-> reclaim_pending
control:     agent <-> user <-> waiting_for_user
```

All commands for one Browser Session are serialized. Commands for separate sessions may progress concurrently, while WebKit calls still execute on the main actor. A WebKit content-process crash invalidates Element References, fails the active command with `browser_process_terminated`, and recreates the view with the shared Browser Profile. bagent never replays the last click, key press, or submission automatically.

## Security and privacy

- Page text, console messages, network summaries, and dialog text are untrusted input.
- Navigation validation occurs before every top-level navigation and after every redirect.
- DNS names pass only when every A and AAAA answer is inside the agreed ranges. Mixed answers fail closed.
- The allowlist is not described as a complete network sandbox because subresources and page-created connections remain unrestricted in v1.
- Password values, cookies, local storage, session storage, authorization headers, and form values never appear in MCP output or audit records.
- Screenshot bytes and Page Snapshot text remain in memory only for the active response.
- Audit entries contain timestamp, connection label, tool name, origin without query or fragment, result class, and redacted argument shape. They contain no page content, typed text, scripts, headers, or images.
- Release builds keep `WKWebView.isInspectable` disabled. A future explicit developer setting may enable it.
- The browser requests neither Screen Recording nor Accessibility for snapshots or semantic actions. If native input eventually requires Accessibility, it remains an optional, separately explained fallback.
- Website permissions, downloads, uploads, clipboard access, arbitrary JavaScript, and profile clearing cannot be enabled by model output.

## Implementation roadmap

### Phase 0: proof gates and accepted ADRs

Create a disposable signed WebKit harness before adding product UI. It must test:

- hidden and offscreen screenshots after navigation, animation, resize, Retina-scale changes, sleep and wake, and memory pressure;
- screen-sized captures on every attached display without visible pixels, focus movement, Space changes, or app-switching entries;
- shared persistent cookies across several `WKWebView` instances and across app relaunch;
- semantic click and typing on native controls, React-style controls, contenteditable, CodeMirror or Monaco, canvas, shadow DOM, and same-origin frames;
- honest `visible_interaction_required` behavior for cases that need native input;
- Navigation Allowlist handling for direct IPs, allowed DNS names, mixed A and AAAA answers, redirects, and disallowed schemes;
- partial console and network coverage, with documented gaps;
- memory cost for one through five concurrent sessions; and
- WebKit process termination and recovery without replaying mutations.

Phase 0 passes only if hidden viewport and screen-sized screenshots are current and produce no user-visible window behavior. If no supported WebKit technique passes, the embedded implementation stops. Safari 27 MCP remains a separate fallback, but it does not satisfy this roadmap's popup requirement.

Phase 0 also accepts the second-panel and browser-host ADRs, then updates `CLAUDE.md` and `docs/UI_DESIGN.md` to describe the planned browser exception without weakening the notch rules for other features.

### Phase 1: browser domain and manual popup

Add the shared Browser Profile, Browser Session registry, four-session limit, state transitions, navigation policy, process recovery, and redacted audit model. Then add one WebKit view and BrowserPanel per session, the origin overlay, manual input preemption, frame persistence, and profile clearing.

Extend the existing left-wing stacked icon deck with Browser Cues. Cover steady, active, attention, detached, and reclaim-pending states. Clicking toggles the matching popup; dragging reveals and places it; the context action closes it.

### Phase 2: local broker and read-only MCP slice

Add the user-only Unix socket and framed Swift protocol. Add the `bagent-browser-mcp` Rust executable, MCP initialization, implicit session ownership, app launch and retry, bounded deadlines, and bundle packaging.

Ship the first vertical slice with `browser_open`, `page_info`, `navigate_to_url`, `get_page_content`, `screenshot`, `set_viewport_size`, `browser_set_visibility`, `browser_console_messages`, and `list_network_requests`. Verify both Codex and Claude against the bundled executable, including automatic hidden inspection for UI tasks and `browser_disabled` when the setting is off.

### Phase 3: controlled interaction

Add Page Snapshot revisions, Element References, `page_interactions`, waits, alert acknowledgement, Control Lease transitions, manual preemption, and `visible_interaction_required`. Add Submission Grants and password-field blocking before enabling form submission.

### Phase 4: parallel sessions and handoff

Enable four concurrent Agent Connections, private session routing, shared-profile behavior, multiple visible popups, detached sessions, user-approved reclaim, and user-approved close requests. Add concurrency, disconnect, reconnect, and crash tests.

### Phase 5: hardening and release gate

Run the complete acceptance matrix in a signed bundle. Add setup instructions and development commands for both MCP clients. Ship behind an explicit bagent Browser setting first. When disabled, the user-only socket returns `browser_disabled` without creating sessions or cues; the setting does not weaken navigation or privacy policy.

Remove the opt-in label only after the hidden-capture, multi-session, security, packaging, and client-compatibility gates pass on the oldest supported macOS release and the current development macOS release.

## Expected code map

Swift additions under `apps/macos/Sources/bagent/`:

- `BrowserCoordinator.swift`
- `BrowserSession.swift`
- `BrowserWindowController.swift`
- `BrowserCommandServer.swift`
- `BrowserNavigationPolicy.swift`
- `BrowserPageBridge.swift`
- `BrowserModels.swift`

Existing Swift files likely to change:

- `AppDelegate.swift`
- `ChatViewModel.swift`
- `ChatView.swift`
- `NotchWindowController.swift`
- `NotchSettingsContent.swift`
- `Package.swift`
- `Info.plist`

Rust and packaging additions:

- new workspace crate `crates/browser_mcp/` producing `bagent-browser-mcp`;
- root `Cargo.toml` workspace membership;
- `apps/macos/Makefile` build, copy, and signing steps; and
- app-bundle verification covering both executables.

Keep browser policy and state transitions in testable types. `BrowserCoordinator` coordinates them but must not become a second monolithic `ChatViewModel`.

## Acceptance criteria

The feature is ready only when all of these are demonstrated in a signed app bundle:

1. A Codex connection and a Claude connection can each own a private session at the same time, while both share a manually established login cookie.
2. A fifth live connection receives `session_limit_reached` without affecting the existing four.
3. Disconnecting leaves a Detached Session running, and another connection cannot inspect it before user-approved reclaim.
4. The popup is hidden by default. Hidden viewport and screen-sized screenshots show current pixels without visible UI, focus loss, Space changes, or persistent files.
5. Dragging each stacked Browser Cue reveals the correct popup. Several popups can remain visible and independently resizable.
6. Manual input revokes only that session's Control Lease, cancels queued actions, and returns `control_revoked_by_user` to its owner.
7. Direct IP, DNS-equivalent, redirect, mixed-answer, disallowed-range, and disallowed-scheme navigation tests match the agreed policy.
8. Page Snapshots are bounded, omit password values, mark cross-origin frames opaque, and reject stale Element References.
9. The supported interaction fixture suite passes. Unsupported native interactions return `visible_interaction_required` without revealing the browser.
10. Submission is blocked until the user grants the session and origin. The approval text states that destructive submissions are included.
11. Camera, microphone, screen sharing, location, notifications, authentication hardware, downloads, uploads, clipboard, popup, and arbitrary-JavaScript attempts fail with the documented result.
12. Console and network tools label coverage as partial and never return prohibited bodies, headers, cookies, or credentials.
13. Closing a session preserves the Browser Profile. Clearing the Browser Profile is user-only, closes all sessions, and removes the test cookie.
14. Quitting removes sessions and cues. Relaunch restores Browser Profile cookies but not pages or ownership.
15. Codex and Claude Code use the same MCP routing policy: explicit bagent-browser requests call `browser_open` first; UI tasks with a known local or allowlisted URL automatically inspect hidden state with `get_page_content` and `screenshot`; non-UI coding does not open a session.
16. The disabled setting returns structured `browser_disabled` with enable instructions, while the proxy still returns `browser_app_unavailable` when the app cannot start.
17. Codex and Claude can launch the bundled stdio proxy, receive image results, survive bagent startup delay, and get structured errors when the app is unavailable.
18. Audit and filesystem inspection find no screenshot bytes, page text, typed values, password values, cookie values, or authorization headers.

## Deferred work

- editable Navigation Allowlist settings;
- tabs and popup windows inside a Browser Session;
- full-page scroll-and-stitch screenshots;
- arbitrary JavaScript developer mode;
- file upload, download, clipboard, and drag-and-drop policy;
- website permissions;
- complete console and network inspection;
- restored live sessions after app restart; and
- optional Safari 27 MCP provider for real-Safari testing.

## Definition of done

The feature is done when the signed bundle passes Phase 0 and all sixteen acceptance criteria, both target clients use the packaged MCP proxy without hand-edited runtime paths, the glossary and ADRs match the shipped behavior, and `CLAUDE.md`, `docs/UI_DESIGN.md`, `docs/SECURITY.md`, and `docs/CONNECTORS.md` describe the new boundaries accurately.
