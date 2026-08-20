# bagent Browser implementation phase results

The accepted Phase 0 gate passed before product implementation. The slices
below were implemented in roadmap order and kept buildable after each slice.

## Phase 0 — proof gate

Passed. See [agent_browser_phase0_results.md](agent_browser_phase0_results.md).
The signed harness proved hidden viewport and screen-sized snapshots without a
listed window or frontmost-process change, plus the named persistent cookie
store across relaunch.

## Phase 1 — browser domain and manual panel

Implemented and build-tested:

- fixed bagent-owned persistent WebKit profile;
- four-session registry and orthogonal runtime/page/visibility/ownership/control
  state machine;
- allowlist policy and redacted audit model;
- hidden, resizable, floating Browser Panel with drag strip, origin overlay,
  cue state, manual-input/resize preemption, profile clearing, dialogs, popup
  blocking, media denial, download denial, and process recovery policy.

Relevant tests: `BrowserSessionStateTests`, `BrowserProfileTests`, and
`BrowserNavigationPolicyTests`.

## Phase 2 — local broker and MCP read path

Implemented and signed-bundle tested:

- user-only Unix socket with `0700` parent, `0600` socket, peer UID check,
  bounded length-prefixed frames, stale-listener handling, and main-actor
  dispatch;
- bundled `bagent-browser-mcp` stdio proxy with bounded startup retry;
- `browser_open`, `page_info`, semantic snapshots, viewport/screen PNG
  responses, visibility, viewport, partial console, and partial network tools.

The packaged proxy launched the signed app, opened the local fixture while
hidden, and returned an MCP `image/png` content block. The unavailable-app path
returned structured `browser_app_unavailable`.

## Phase 3 — controlled interaction

Implemented and fixture-tested:

- revision-scoped semantic Element References and bounded snapshots;
- DOM click/type/press/hover/focus/scroll actions;
- visible coordinate click/move/scroll path, stale-reference rejection,
  control revocation, navigation deadlines, and waits;
- session-and-origin Submission Grants, password boundary, native-input
  boundary, close approval, detached-session reclaim approval, and reclaim
  rejection.

The signed fixture returned `submission_grant_required` with the explicit
destructive-submission warning, `permission_not_supported` for a camera
attempt, and an image result for the hidden screenshot path.

## Phase 4 — quality and hard-deny boundaries

Implemented:

- explicit `coverage: partial` console/network results;
- URL/query/fragment redaction and failure-reason redaction;
- no bodies, headers, cookies, storage, credentials, page values, raw HTML,
  arbitrary JavaScript, uploads, downloads, clipboard, popups, or unsupported
  website permissions;
- injected page-side denials plus native WebKit media/download delegate denials.

## Phase 5 — packaging, settings, and documentation

Implemented:

- explicit bagent Browser setting and user-confirmed profile clear action;
- packaged localization catalog;
- Codex/Claude MCP transport documentation;
- nested signing for `bagent`, `bagentd`, and `bagent-browser-mcp`;
- signed-bundle verification with all three executables present.

The remaining platform-only checks are documented rather than guessed:
Stage Manager/Mission Control permutations, sleep/wake, real Codex and Claude
client registration, and a manufactured WebKit content-process crash require
interactive or external test conditions not available to the repository test
suite.
