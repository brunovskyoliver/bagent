# Spike: an agent-controlled WebKit browser for bagent

**Date:** 2026-08-19  
**Status:** research only, no implementation  
**Target:** macOS 14+, with the current development Mac on macOS 26.5.2 and Safari 26.5.2

## Decision in one page

Build a bagent-owned browser around `WKWebView`. Keep the view and its persistent `WKWebsiteDataStore` in the Swift app, place it in a second AppKit panel, and expose it to Codex and Claude through a bundled stdio MCP server. The MCP process should be a thin Rust proxy. It connects to the running bagent app over a user-only Unix domain socket and turns MCP calls into serialized browser commands on the Swift main actor.

Call it **bagent Browser** in the product. It uses Apple's WebKit engine, but it is not Safari.app. It cannot mount the user's normal Safari profile through a supported public API, so it will not inherit Safari cookies, history, AutoFill data, content blockers, or installed Safari extensions. `WKWebsiteDataStore` explicitly owns a web view's cookies and caches, and Apple provides separate persistent stores for browser profiles. Apple points apps that need Safari-backed sign-in sharing to `ASWebAuthenticationSession`, not to Safari's profile directory. This separation is a feature for agent safety, but the user will need to sign in again inside the popup. Sources: [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview), [WKWebsiteDataStore](https://developer.apple.com/documentation/webkit/wkwebsitedatastore), [Safari Services](https://developer.apple.com/documentation/safariservices), and [ASWebAuthenticationSession](https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession).

Apple now has its own Safari MCP server, but only in Safari 27 and later. It can inspect the DOM, capture screenshots, observe console and network activity, and simulate page input. Apple configures it as a stdio server with `/usr/bin/safaridriver --mcp`. On this Mac, `/usr/bin/safaridriver --help` has no `--mcp`, which agrees with the installed Safari 26.5.2 version. Even after an upgrade, Safari MCP opens a controlled Safari window with Safari UI and a control banner. It does not provide the chromeless, draggable in-bagent panel requested here. Treat it as a future optional backend and as the naming/behavior reference for bagent's MCP tools. Source: [Connecting an AI agent to Safari](https://developer.apple.com/documentation/safari-developer-tools/connecting-an-ai-agent-to-safari).

Two spikes must pass before the feature is scheduled:

1. Verify that `WKWebView.takeSnapshot` returns current pixels while its window is removed from the screen with `orderOut`, after navigation, animation, resize, and sleep/wake. Apple documents snapshot capture and documents that `orderOut` hides without releasing the window, but it does not promise headless or offscreen rendering. Sources: [`takeSnapshot`](https://developer.apple.com/documentation/webkit/wkwebview/takesnapshot%28with%3Acompletionhandler%3A%29) and [`orderOut`](https://developer.apple.com/documentation/appkit/nswindow/orderout%28_%3A%29).
2. Compare DOM-driven actions with native synthetic input on real sites. Public `WKWebView` has JavaScript execution, but no WebDriver endpoint for embedded views and no public equivalent of Safari MCP's `page_interactions`. JavaScript `element.click()` does not satisfy every trusted-user-gesture check. Synthetic AppKit events may also differ from physical input. Authentication, popups, drag and drop, editors, file inputs, and payment controls are the important cases. Sources: [`evaluateJavaScript`](https://developer.apple.com/documentation/webkit/wkwebview), [Safari WebDriver](https://developer.apple.com/documentation/safari-developer-tools/webdriver), and [Safari MCP tools](https://developer.apple.com/documentation/safari-developer-tools/connecting-an-ai-agent-to-safari).

If either spike fails, do not disguise the limitation. Keep the panel for manual browsing and screenshots, then use Safari 27 MCP for high-fidelity automation when available. A Chromium helper would be another fallback, but it would no longer test WebKit and is outside this proposal.

## Fit with the current codebase

bagent already has the right process split for this feature:

- The macOS process owns AppKit and SwiftUI. [`NotchWindowController.swift`](../../apps/macos/Sources/bagent/NotchWindowController.swift) already handles panel geometry, key-window behavior, all-spaces placement, and global/local event monitors.
- The Rust daemon is resident and already uses authenticated local IPC, SSE, typed tools, audit records, and approval gates. See [`ARCHITECTURE.md`](../ARCHITECTURE.md) and [`main.rs`](../../crates/daemon/src/main.rs).
- The repository already uses `rmcp` as an MCP client for Odoo. The new direction is the inverse: a small MCP server executable that agents launch. See [`crates/connectors/odoo/src/mcp.rs`](../../crates/connectors/odoo/src/mcp.rs).
- The existing screen path captures the whole display with ScreenCaptureKit and reduces it to OCR. Browser screenshots should not use that path. `WKWebView.takeSnapshot` produces the web view image directly and keeps the browser capture independent of Screen Recording permission. The first spike should confirm this in the signed bundle. See [`ScreenContextProvider.swift`](../../apps/macos/Sources/bagent/ScreenContextProvider.swift).

There is one deliberate product break. [`UI_DESIGN.md`](../UI_DESIGN.md) and [`CLAUDE.md`](../../CLAUDE.md) say the notch is the only UI and that bagent has one panel. The browser is a real second panel, not another notch state. Before implementation, record this exception in an ADR and update those documents. Trying to squeeze an interactive browser into the current fixed notch panel would couple browser focus, resizing, and lifetime to chat state and would make both surfaces worse.

## Proposed architecture

```text
Codex / Claude
      |
      | MCP over stdio
      v
bagent-browser-mcp                  bundled Rust executable
      |
      | length-prefixed local RPC
      | ~/Library/Application Support/bagent/browser.sock (0600)
      v
BrowserCommandServer                Swift, user-only Unix socket
      |
      | one serialized command queue
      v
@MainActor BrowserCoordinator
      |-- BrowserWindowController
      |      `-- BrowserPanel + WKWebView
      |-- BrowserSession / ownership lease
      `-- policy, redacted audit events, in-memory image replies
```

Use a Unix domain socket rather than adding a fixed localhost port. `NWEndpoint` has a public Unix-path endpoint type, and a local proxy avoids putting the daemon's bearer token or a changing port in Codex and Claude configuration. The implementation may use Network.framework where its listener API is sufficient, or a small POSIX `AF_UNIX` listener in Swift. The socket's parent directory should be mode `0700`, the socket mode `0600`, and the server should reject peers whose effective UID differs from bagent's. Remove a stale socket only after confirming that no listener owns it. Source for the public endpoint type: [`NWEndpoint.unix(path:)`](https://developer.apple.com/documentation/network/nwendpoint).

The proxy should contain protocol code, discovery, and framing only. Browser state belongs in Swift because every `WKWebView` operation is main-actor UI work. Keep audit persistence in the daemon by posting small, redacted events from Swift. Do not put screenshot bytes, page text, form values, cookies, query strings, or evaluated script bodies in the audit database.

The proxy should try the socket, launch `sk.bagent.app` in the background if it is absent, and retry for a short bounded interval. It must report `browser_app_unavailable` if startup fails. It should never start a second independent browser process.

Both target clients support local stdio MCP servers. Codex supports stdio and Streamable HTTP and stores the configuration under `[mcp_servers.<name>]`; Claude Code describes stdio servers as the local option for tools needing direct system access. Sources: [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) and [Claude Code MCP configuration](https://code.claude.com/docs/en/mcp).

Example setup after installation:

```bash
codex mcp add bagent-browser -- "/Applications/bagent.app/Contents/MacOS/bagent-browser-mcp"
claude mcp add bagent-browser -- "/Applications/bagent.app/Contents/MacOS/bagent-browser-mcp"
```

MCP's stdio transport requires newline-delimited JSON-RPC on stdout and reserves stderr for logs. The proxy must emit nothing else on stdout. Source: [MCP transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

### Swift browser ownership

Add a `BrowserCoordinator` owned by `AppDelegate`, alongside the existing notch controller. It creates one `WKWebView` with:

- a named persistent `WKWebsiteDataStore` identifier, saved in `UserDefaults`, so the bagent browser keeps its own logins across launches;
- a `WKNavigationDelegate` and `WKUIDelegate` for navigation policy, authentication challenges, popups, JavaScript dialogs, and downloads;
- a `WKUserContentController` for the small injected agent bridge;
- `isInspectable = true` only in debug builds or behind an explicit developer setting. It defaults to false and exposes the view to Safari Web Inspector when enabled. Sources: [`WKWebsiteDataStore`](https://developer.apple.com/documentation/webkit/wkwebsitedatastore), [`WKUserContentController`](https://developer.apple.com/documentation/webkit/wkusercontentcontroller), and [`isInspectable`](https://developer.apple.com/documentation/webkit/wkwebview/isinspectable).

Use one profile and one tab in v1. A tab model can come later. Multiple hidden tabs increase WebKit process and memory behavior before the basic contract is proven.

### Popup behavior

Use a separate `BrowserPanel` with style masks `[.titled, .resizable, .fullSizeContentView]`. Make the title bar transparent, hide its title and standard buttons, and let the web view fill the content. This preserves AppKit's real edge resizing while removing visible browser chrome. AppKit documents `.resizable`, `.fullSizeContentView`, and transparent title bars as public window behavior. Sources: [`NSWindow.StyleMask`](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct) and [`titlebarAppearsTransparent`](https://developer.apple.com/documentation/appkit/nswindow/titlebarappearstransparent).

Reserve a narrow drag strip at the top of the panel. It can be visually transparent until hover, but must remain discoverable and must not cover page controls without feedback. Pass its mouse-down event to `NSWindow.performDrag(with:)`, which lets the Window Server handle Spaces and normal window movement. Keep standard resize hit regions on all edges. Source: [`performDrag(with:)`](https://developer.apple.com/documentation/appkit/nswindow/performdrag%28with%3A%29).

Set the panel level to `.floating`, not `.statusBar`. AppKit describes floating level as appropriate for palettes and places it above normal windows. The existing notch can remain at `.statusBar`. Use `[.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]`, then test Stage Manager, full-screen apps, and multiple displays. These collection behaviors are public, but Apple notes that some apply differently across Spaces, full screen, and Stage Manager. Sources: [`NSWindow.Level`](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct), [`level`](https://developer.apple.com/documentation/appkit/nswindow/level-swift.property), and [`NSWindow.CollectionBehavior`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct).

Hidden means `orderOut(nil)`, not zero opacity and not a visible 1-point window. Keep the panel and `WKWebView` strongly owned so the page/profile survives. When the user drags the browser handle out of the notch, show the already-owned panel under the pointer and hand movement to AppKit. Do not move the web view between the notch and browser windows. When the user dismisses it, call `orderOut` and retain the page. Apple says `orderOut` removes a window from the screen list without releasing it. Source: [`orderOut`](https://developer.apple.com/documentation/appkit/nswindow/orderout%28_%3A%29).

The popup has no URL bar, back button, tabs, or toolbar. For phishing resistance, show the current origin in the notch browser handle and in a temporary overlay when the panel becomes interactive, when the top-level origin changes, or when a password field receives focus. This is not navigation chrome. It is the minimum cue needed before the user types into a chromeless web page.

## Agent interaction model

### Page snapshot and element references

Inject a small script in an isolated `WKContentWorld`. It should return a bounded semantic snapshot of the main frame: URL, title, visible text, headings, landmarks, forms, and interactive elements with role, accessible name, state, and viewport bounds. Give interactive nodes opaque references such as `e_42`. Invalidate all references on top-level navigation and stamp each response with `page_revision`. Reject calls that use a stale revision instead of guessing.

`WKUserContentController` is Apple's bridge for injected scripts and native code, and content worlds scope those scripts away from page JavaScript. `WKWebView` can also evaluate or call asynchronous JavaScript in a chosen frame and content world. Sources: [`WKUserContentController`](https://developer.apple.com/documentation/webkit/wkusercontentcontroller) and [`WKWebView` JavaScript APIs](https://developer.apple.com/documentation/webkit/wkwebview).

This is a DOM-derived semantic snapshot, not WebKit's private accessibility tree. Label it accurately in MCP output. Cross-origin iframes are opaque to page JavaScript and need to appear as frames with bounds and origin, not silently disappear.

### Screenshots

Use `WKWebView.takeSnapshot` and return a PNG as MCP image content in the same tool result. MCP tool results support base64 image blocks with a MIME type. Keep the `NSImage`/PNG only long enough to frame the response. Sources: [`takeSnapshot`](https://developer.apple.com/documentation/webkit/wkwebview/takesnapshot%28with%3Acompletionhandler%3A%29) and [MCP tool image content](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

Ship viewport screenshots first. `WKSnapshotConfiguration.rect` must stay within the web view's bounds, so a full-page image is not a single documented API call. A later implementation can scroll and stitch with fixed-position-element handling, or return a PDF for long-page review. Source: [`WKSnapshotConfiguration.rect`](https://developer.apple.com/documentation/webkit/wksnapshotconfiguration/rect).

### Input

Use two paths, with the first spike deciding their exact boundary:

1. Semantic actions resolve a fresh element reference and invoke the relevant DOM operation. Prefer focus plus value/input/change events for text and a real `HTMLElement.click()` for ordinary controls.
2. Coordinate actions translate CSS-pixel bounds into the `WKWebView` coordinate system and send AppKit mouse or key events to the view while its panel is key. This path is for canvas, editors, hover, and hit-testing.

Never silently fall back from one path to the other. Return the method used, final URL, page revision, and whether navigation began. A successful JavaScript return is not proof that the site's action completed, so agents should follow mutations with `wait_for_navigation`, `page_info`, `get_page_content`, or a screenshot.

Public `WKWebView` does not expose WebDriver-grade automation for embedded views. Safari WebDriver targets isolated Safari automation windows, permits only one session at a time, and places a glass pane over those windows. It is not an API for driving bagent's `WKWebView`. Source: [Safari WebDriver](https://developer.apple.com/documentation/safari-developer-tools/webdriver).

### Console and network limits

Do not promise parity with Chrome DevTools or Safari 27 MCP in v1. Public `WKNavigationDelegate` reports navigation lifecycle, not every request and response. JavaScript instrumentation can wrap `console`, `fetch`, and `XMLHttpRequest`, and `PerformanceResourceTiming` can provide partial resource timing, but it will miss or redact cross-origin data, service-worker traffic, cached work, and some browser-internal requests. Likewise, console wrapping can miss messages emitted before injection or from inaccessible frames.

Name these tools `browser_console_messages` and `list_network_requests` only after their output clearly says `coverage: partial`. Safari 27 MCP is the supported path for full Safari console and network inspection; Apple lists both capabilities explicitly. Source: [Safari MCP tool list](https://developer.apple.com/documentation/safari-developer-tools/connecting-an-ai-agent-to-safari).

## MCP contract proposal

Mirror Safari 27 names where bagent can match their meaning. Use a `browser_` prefix for common nouns that would otherwise collide with other MCP servers.

| Tool | Core input | Result | MCP annotation / bagent policy |
|---|---|---|---|
| `browser_open` | `url?`, `viewport?`, `profile="default"` | session, page info, ownership lease | Not read-only; may navigate; always hidden |
| `navigate_to_url` | `session_id`, `url`, `wait="load"`, `timeout_ms?` | final URL, title, status, revision | Not read-only; origin policy applies |
| `page_info` | `session_id` | URL, title, loading state, viewport, visibility, revision | `readOnlyHint: true` |
| `get_page_content` | `session_id`, `format="semantic"|"markdown"|"html"`, `max_chars?` | bounded content and opaque element refs | `readOnlyHint: true`; page data is untrusted |
| `page_interactions` | `session_id`, `revision`, `actions[]` | one result per action, final page info | Not read-only; serial, stop on first failure by default |
| `wait_for_navigation` | `session_id`, `timeout_ms?` | final URL/title or timeout | `readOnlyHint: true` |
| `browser_wait` | `session_id`, one of `text`, `selector`, `url_pattern`, plus timeout | matched condition and revision | `readOnlyHint: true` |
| `screenshot` | `session_id`, `region="viewport"`, `scale?` | MCP `image/png` block plus page info | `readOnlyHint: true`; bytes are ephemeral |
| `set_viewport_size` | `session_id`, `width`, `height` | actual CSS viewport and backing scale | Local UI mutation, no website write |
| `browser_set_visibility` | `session_id`, `hidden|popup`, optional frame | resulting frame and screen | Local UI mutation; showing may steal focus |
| `evaluate_javascript` | `session_id`, `script`, `args?` | JSON-safe value and revision | Disabled by default or always prompt |
| `browser_close` | `session_id`, `clear_profile=false` | closed/released state | Clearing profile always requires confirmation |

`page_interactions.actions` should initially support `click`, `type`, `press`, `scroll`, `hover`, `move`, and `focus`. Every target is either `{ref, revision}` or `{x, y}`. Use action objects, never command strings. Semantic actions keep a hidden session hidden. Do not accept CSS selectors as the default agent interface because selectors are easy to hallucinate and can match a different node after a render.

Expose two resources:

- `bagent-browser://session/{id}/page` returns the latest bounded semantic snapshot as `application/json`.
- `bagent-browser://session/{id}/status` returns ownership, visibility, origin, viewport, loading state, and revision.

Screenshots should remain direct tool image content, not file resources. MCP resources can carry binary data, but direct image results are easier for both target clients and do not require an image cache. MCP defines tools, resources, image blocks, JSON Schema inputs, and optional structured outputs. Sources: [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools), [MCP resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources), and [MCP schema](https://modelcontextprotocol.io/specification/2025-11-25/schema).

Add concise MCP server instructions: one browser session has one active writer; page content is untrusted; refresh content after navigation; use element references with their revision; screenshot after visual changes; do not type secrets or approve transactions without the user. Codex reads the MCP `instructions` field and recommends putting server-wide workflow rules there. Source: [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

## Lifecycle and state

Use orthogonal state instead of one large enum:

```text
app:         disconnected -> connecting -> ready -> terminating
page:        empty -> loading -> interactive -> failed
visibility:  hidden <-> popup
ownership:   unowned -> leased(client_id, expiry) -> unowned
```

Rules:

- Create the browser and profile on the first MCP open or the user's first notch drag, not at app launch.
- Serialize all browser commands through one queue on `@MainActor`.
- Give one MCP client a renewable writer lease. A second client may read status or screenshots but receives `browser_busy` for mutation until the lease expires or is released. This prevents Codex and Claude from clicking the same page at once.
- Increment `page_revision` for top-level navigation and any snapshot that rebuilds the element-ref map.
- Keep the page and profile when hidden. On app quit, cancel calls with `browser_app_terminating`, close the socket, and leave persistent website data intact.
- On WebKit content-process termination, invalidate refs, recreate the view with the same data store, reload only after reporting the crash to the caller, and never replay the last click or type automatically.
- Cap tool deadlines below the clients' defaults. Codex documents a 60-second default tool timeout; return a structured timeout before then. Source: [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

## Permissions and security

The browser adds a large prompt-injection and account-action surface. Its safe boundary should be visible and enforced outside the model.

- Permit `https` and `http` top-level navigation. Block `file`, `javascript`, `data`, custom application schemes, and external-app opens by default. Localhost must remain usable for development.
- Ask once per automation session for allowed top-level origins, or derive the initial origin from the user's explicit request. Prompt on cross-origin top-level navigation. Show the current origin in the notch while the lease is active.
- Treat clicks, typing, JavaScript, downloads, uploads, permission prompts, password fields, and form submission as side-effecting. MCP recommends a human-in-the-loop interface for tool calls. Source: [MCP tool safety guidance](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
- Block file pickers and downloads in v1. They cross the web/process boundary and need a separate path policy, quarantine behavior, and approval design.
- Never expose cookie values, local storage, AutoFill, Keychain data, request authorization headers, or password-field values through MCP.
- Mark all page-derived text as untrusted in tool descriptions and server instructions. The MCP client model will see attacker-controlled pages.
- Show an unmistakable notch indicator while any MCP client holds control, with a one-click stop that revokes the lease and cancels queued mutations.
- Audit client ID, tool, timestamp, origin with query removed, success/error class, and hashed argument shape. Do not audit screenshot bytes, DOM text, typed text, script source, headers, or URLs containing query/fragment data.

The current bundle is signed but not App Sandboxed, and its Makefile does not enable Hardened Runtime. If bagent later enables App Sandbox, the web view needs the outgoing-network client entitlement. Apple says `com.apple.security.network.client` allows a sandboxed app to initiate outgoing connections. Developer ID notarization requires Hardened Runtime, but App Sandbox is optional outside the Mac App Store. Sources: [`com.apple.security.network.client`](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.network.client), [preparing an app for distribution](https://developer.apple.com/documentation/xcode/preparing-your-app-for-distribution), and [notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

Do not request Accessibility or Screen Recording solely for this browser. Manual mouse and keyboard input arrives through the app's own key window. View screenshots come from WebKit. If the synthetic-input spike proves that global `CGEvent` injection is required, make Accessibility a separately explained fallback and keep semantic DOM actions available without it. Apple's trust API confirms that Accessibility client access is a user-granted capability. Source: [`AXIsProcessTrustedWithOptions`](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions).

## Implementation phases

### Phase 0: proof and ADR

- Write the ADR that makes the browser panel an exception to the notch-only rule.
- Build a disposable signed `WKWebView` harness with a named persistent profile.
- Test `orderOut` snapshots, animations, video/canvas, resize, Retina scale, sleep/wake, WebKit process death, and memory pressure.
- Test DOM click/type against native events on a local fixture suite and real examples: Monaco/CodeMirror, contenteditable, canvas, OAuth login, popup, shadow DOM, cross-origin iframe, drag and drop, and a trusted-user-gesture API.
- Measure whether hidden pages throttle timers or fail waits. Record the result instead of assuming headless behavior.

Exit only if hidden viewport screenshots are current and the supported interaction subset is honest and repeatable.

### Phase 1: manual browser panel

- Add `BrowserCoordinator`, `BrowserWindowController`, one `WKWebView`, persistent profile, hidden/popup state, drag-from-notch behavior, resize, origin cue, and stop control.
- Handle top-level navigation, dialogs, new-window requests, process termination, and app quit.
- Keep downloads, uploads, media capture, geolocation, and notifications blocked.

### Phase 2: local broker and MCP read path

- Add the user-only socket, peer checks, framing, bounded messages, stale-socket handling, and startup retry.
- Add the bundled stdio MCP proxy and configure both Codex and Claude in development.
- Ship `browser_open`, `page_info`, `get_page_content`, `screenshot`, `set_viewport_size`, `browser_set_visibility`, and resources.
- Return screenshot images in memory. Add redacted audit events and concurrent-client tests.

### Phase 3: controlled interaction

- Add element refs/revisions, `page_interactions`, waits, navigation, and writer leases.
- Add origin policy, approval gates, control indicator, emergency stop, deadlines, and cancellation.
- Keep `evaluate_javascript` behind an explicit developer toggle and approval.

### Phase 4: quality and optional Safari 27 adapter

- Add partial console/network capture with explicit coverage metadata only if it proves useful.
- Add scroll-and-stitch screenshots after fixed/sticky element and long-page tests pass.
- On Safari 27+, offer Apple's `/usr/bin/safaridriver --mcp` as a separate provider for true Safari testing, console/network detail, and higher-fidelity page interactions. Do not silently switch providers because profiles and window behavior differ.

## Main risks and open questions

| Risk or question | Current answer |
|---|---|
| Can a hidden `WKWebView` render and snapshot reliably? | Not guaranteed by public docs. Phase 0 is a hard gate. |
| Is this the user's Safari? | No. It is WebKit with a bagent-owned profile. The product name must say so. |
| Can it reuse Safari logins/extensions? | No supported general profile-sharing path. Sign in manually in bagent Browser; use `ASWebAuthenticationSession` only for designed OAuth/SSO flows. |
| Are agent clicks equivalent to physical clicks? | Not always. DOM and synthetic events need a published compatibility boundary. |
| Can it provide DevTools network and console data? | Only partial instrumentation with public embedded-view APIs. Safari 27 MCP is the better provider. |
| Can it take a whole-page PNG in one call? | No documented snapshot rectangle outside view bounds. Start with viewport PNG. |
| What if Codex and Claude connect together? | One renewable writer lease; read calls can coexist. |
| Can a page steal secrets from the agent? | Page content is untrusted. Never expose browser stores or secret fields, constrain origins, and keep sensitive actions behind the user. |
| Does always-on-top mean above the notch and system UI? | No. Use `.floating`; reserve `.statusBar` for the existing notch. |
| What happens without bagent running? | The stdio proxy launches the app in the background, waits briefly, then returns a structured error. |
| Should Safari 27 MCP replace this later? | No. It is an optional real-Safari backend with different UI and profile semantics. |

## Recommendation

Proceed with Phase 0 and the ADR. The overall design is feasible with public APIs, and it fits bagent's Swift/AppKit front end well. The weak point is automation fidelity, not the popup. A `WKWebView` gives bagent a clean WebKit rendering surface, persistent isolated logins, direct screenshots, and normal manual interaction. It does not give an embedded app Safari's private automation machinery. The first implementation should stay narrow: one profile, one page, hidden or floating popup, viewport screenshots, semantic page content, and a small tested action set. Safari 27 MCP can cover the cases where the user needs Safari itself.
