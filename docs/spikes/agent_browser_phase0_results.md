# Phase 0 WebKit harness results

Run from the repository root:

```sh
scripts/run-agent-browser-phase0.sh
```

The script builds the disposable `bagent-browser-harness` executable in release
mode, places it in `Phase0Harness.app`, signs the app ad hoc, verifies the
signature, starts the local fixture server, and runs the three harness modes.
The harness uses public `WKWebView`, `WKSnapshotConfiguration`, AppKit
`NSWindow.orderOut`, and a named persistent `WKWebsiteDataStore`.

Run date: 2026-08-19

## Exit decision

Phase 0 passed its hard screenshot gate. Hidden viewport and screen-sized
captures were current and did not list the hidden window, change the
frontmost process, or activate a visible app window. The harness used
`afterScreenUpdates = true`; this is the configuration that passed the gate.

The output below is the reproducible evidence from the successful run.

```json
{"harness":"bagent-browser-phase0","mode":"all","passed":true,"results":[{"details":{"animated_sha256":"aabbd2ce0ed4cc40abe3c4fb3b7becbc613eb8ec6af5b6ac8a145ec3489584b5","capture_method":"WKWebView.takeSnapshot while NSWindow.orderOut","frontmost_pid_unchanged":"true","initial_sha256":"2e5b37f716464c733d2a893fcc0f71871f906ff31981f88a25d843c5a787feff","resized_sha256":"d59b6f1d226cd9d897a72ba60e20c94d602d1de6287aaab399d3059c4deaf0f8","restored_sha256":"aabbd2ce0ed4cc40abe3c4fb3b7becbc613eb8ec6af5b6ac8a145ec3489584b5","window_on_screen":"false"},"name":"hidden_viewport_snapshots","passed":true},{"details":{"display":"C32HG7x","frontmost_pid_unchanged":"true","sha256":"2bbe1d3f85d6478d1abf7c0a949e1a08dddab45e5eb5a2e11f1fd79da0731726","space_observation":"no public API; no activation or visible window was observed","visible_frame_points":"2560x1410","window_on_screen":"false"},"name":"screen_capture_0","passed":true},{"details":{"method":"DOM semantic actions with explicit input/change dispatch","result":"{\"input\":\"typed\",\"native\":\"yes\",\"shadow\":\"yes\",\"editable\":\"edited\",\"canvas\":\"yes\",\"frame\":\"same-origin frame\"}","trusted_user_gesture":"not claimed; return visible_interaction_required when a site requires native input"},"name":"semantic_fixture_interactions","passed":true},{"details":{"allowlist":"127.0.0.0/8, ::1, 172.19.0.0/16, 172.29.0.0/16","fixture_cases":"direct IP, allowed names, mixed answers, redirects, disallowed schemes/ranges are covered by BrowserNavigationPolicy tests","mixed_dns":"fail closed when any A or AAAA answer is outside the ranges","redirects":"revalidate in navigation response delegate"},"name":"navigation_policy_fixtures","passed":true},{"details":{"console":"fixture bridge can capture explicit console calls only","coverage":"partial","network":"navigation delegate and PerformanceResourceTiming only; service workers/cache/internal traffic are absent","prohibited":"no bodies, cookies, authorization headers, or complete headers"},"name":"console_and_network_coverage","passed":true},{"details":{"drag_drop_upload_download":"unsupported by design","native_only":"not synthesized while hidden; product must return visible_interaction_required","popup":"new-window policy is blocked by WKUIDelegate","semantic_dom":"repeatable for native, React-style, contenteditable, shadow DOM, canvas state, and same-origin frame fixture"},"name":"native_input_boundary","passed":true},{"details":{"limit":"product limit is four; fifth is a Phase 0 benchmark only","measurement":"Use Instruments Activity Monitor for resident memory; this harness proves the creation path and records no image files","sessions":"1,2,3,4,5 instantiated sequentially in one signed process","snapshot_sha256_count":"5"},"name":"concurrent_session_memory","passed":true},{"details":{"harness_note":"termination callback is platform-driven and is recorded by the product test seam","recovery_policy":"WKNavigationDelegate content-process termination is observable; refs are invalidated and no mutation is replayed"},"name":"process_termination_recovery","passed":true}]}
{"harness":"bagent-browser-phase0","mode":"cookie-write","passed":true,"results":[{"details":{"cookie_present":"true","store":"sk.bagent.phase0"},"name":"persistent_cookie_write","passed":true}]}
{"harness":"bagent-browser-phase0","mode":"cookie-read","passed":true,"results":[{"details":{"cookie_present":"true","store":"sk.bagent.phase0"},"name":"persistent_cookie_relaunch","passed":true}]}
```

## Boundaries recorded by the harness

- The semantic fixture proves repeatable DOM-side changes for native controls,
  a React-style state change, `contenteditable`, shadow DOM, canvas state, and
  a same-origin frame. It does not claim trusted user-gesture equivalence.
- Console and network coverage is partial. The harness records no bodies,
  cookies, authorization headers, or complete headers.
- The five-session memory pass instantiates the creation path and produces
  five snapshots. A resident-memory number requires Instruments and is not
  fabricated here; the product limit remains four.
- WebKit process termination is handled by the product delegate seam. The
  harness does not manufacture a private or unsupported WebKit crash.
- Space changes have no public readback API suitable for a proof assertion.
  The run used no activation or visible window and observed no app-switching
  entry; this limitation remains explicit.
