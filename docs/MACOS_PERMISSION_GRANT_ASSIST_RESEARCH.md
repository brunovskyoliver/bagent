# macOS Permission Grant Assist research

Research for [Verify current macOS Permission Grant Assist mechanics](https://github.com/brunovskyoliver/bagent/issues/16). The repository targets macOS 14 and later. Live checks below cover only macOS 26.5.2 (25F84), Apple silicon, on 2026-07-30; they did not grant, revoke, reset, or request a permission.

## Decision summary

- Keep the four privacy-pane anchors below, but treat the category suffixes as compatibility behavior rather than public API. If a category open cannot be confirmed, fall back to the live-verified Privacy & Security root and show the localized pane name.
- Drag the installed, signed running application as a file URL: `Bundle.main.bundleURL` must be an existing `.app` bundle, and an `NSURL`/`URL` pasteboard writer must provide `public.file-url`. Do not drag an image, executable, alias, file promise, or source-build path.
- Recheck nonprompting probes at initial display and whenever `NSApplication.didBecomeActiveNotification` fires. Prompt APIs are initiation mechanisms, not authoritative completion callbacks.
- Full Disk Access (FDA) has no public grant-status API. Probe the exact protected resources the product needs by actually opening them, report per-resource results, and require the running signed app/helper identity to match the item the user grants. Metadata-only `isReadableFile` is insufficient.
- Use `CGPreflightScreenCaptureAccess()` for Screen Recording and `AXIsProcessTrusted()` for Accessibility. For the microphone row specified by `docs/UI_DESIGN.md`, add `AVCaptureDevice.authorizationStatus(for: .audio)` and the required `NSMicrophoneUsageDescription` before offering a request action.
- Offer a UI-only relaunch when FDA or Screen Recording still fails its post-activation probe. Accessibility and microphone normally re-evaluate in process; show relaunch only if the authoritative probe remains false. The exact macOS 14/15/26 mutation-time behavior remains unverified because proving it would require changing TCC state.
- A permission relaunch must launch a replacement UI in an explicit handoff mode that skips `DaemonLauncher.launch()`. Ordinary startup currently reinstalls and restarts the daemon and can also restart BaseRT; that path is not a UI-only relaunch.
- Package a real app icon before presenting the drag affordance. `bagent_icon.png` is currently absent from this worktree and Git history; a sibling checkout has an untracked 1254×1254 RGB PNG with no alpha or color profile. Normalize an approved source to Apple's 1024×1024 square/color-space guidance, generate the macOS size variants (or current Icon Composer/asset-catalog output), place compiled icon resources in `Contents/Resources`, declare the bundle icon, then sign. A standalone PNG may also be bundled for the in-notch illustration, but it is not the drag payload and should not be the only Finder/System Settings icon source.

## Documented findings (Apple primary sources)

### Permission APIs and user control

- Apple says an app cannot gain FDA through an entitlement or code; the user grants it in System Settings. The current Mac User Guide says users add an app in Privacy & Security > Full Disk Access with Add, select, and Open. [App Sandbox file access](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox), [Privacy & Security settings](https://support.apple.com/guide/mac-help/change-privacy-security-settings-on-mac-mchl211c911f/mac)
- `CGPreflightScreenCaptureAccess()` is the nonprompting Screen Recording probe and `CGRequestScreenCaptureAccess()` is the request API. Apple also documents user management under Screen & System Audio Recording. [Preflight](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess%28%29), [request](https://developer.apple.com/documentation/coregraphics/cgrequestscreencaptureaccess%28%29), [user guide](https://support.apple.com/guide/mac-help/allow-apps-to-use-screen-and-audio-recording-mchl592e5686/mac)
- `AXIsProcessTrusted()` returns whether the current process is a trusted Accessibility client. `AXIsProcessTrustedWithOptions` can show the prompt, but Apple explicitly says prompting is asynchronous and does not change that call's return value. [Probe](https://developer.apple.com/documentation/applicationservices/1460720-axisprocesstrusted), [prompt option](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)
- `AVCaptureDevice.authorizationStatus(for: .audio)` is the authoritative microphone status and `requestAccess(for:)` is the request path. A microphone usage description is mandatory before requesting/capturing. [Status](https://developer.apple.com/documentation/avfoundation/avcapturedevice/authorizationstatus%28for%3A%29), [request](https://developer.apple.com/documentation/avfoundation/avcapturedevice/requestaccess%28for%3Acompletionhandler%3A%29), [user guide](https://support.apple.com/guide/mac-help/control-access-to-the-microphone-on-mac-mchla1b1e1fe/mac)
- AppKit posts `NSApplication.didBecomeActiveNotification` immediately after activation, making it the reliable recheck seam after the user returns from System Settings. [AppKit notification](https://developer.apple.com/documentation/appkit/nsapplication/didbecomeactivenotification)

### Drag and identity

- AppKit drag and drop uses the drag pasteboard. `NSURL` implements `NSPasteboardWriting`, and existing files should travel as file URLs rather than file promises. [NSPasteboard](https://developer.apple.com/documentation/appkit/nspasteboard), [NSPasteboardWriting](https://developer.apple.com/documentation/appkit/nspasteboardwriting), [file drag sample](https://developer.apple.com/documentation/appkit/supporting-drag-and-drop-through-file-promises)
- TCC remembers privacy choices using code identity/designated requirements. Apple specifically uses microphone authorization as its example, and an Apple DTS engineer confirms that ad-hoc rebuilds are treated as new identities for Screen Recording. Keep a stable bundle identifier and signing identity across builds; development and Developer ID variants can still have different designated requirements. [TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements), [DTS confirmation](https://developer.apple.com/forums/thread/819406)

### Icons

- Apple documents the app icon as a 1024×1024 square source, with system masking and supported sRGB/P3/Gray color spaces. For macOS asset catalogs, Apple requires assets for each size. `CFBundleIconFile` names the bundle icon file, and `NSWorkspace.icon(forFile:)` returns the icon associated with a bundle. [HIG](https://developer.apple.com/design/human-interface-guidelines/app-icons), [asset catalog](https://developer.apple.com/documentation/xcode/configuring-your-app-icon), [bundle key](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleiconfile), [workspace icon](https://developer.apple.com/documentation/appkit/nsworkspace/icon%28forfile%3A%29)

Apple's public documentation names the panes and provides a Privacy & Security root action, but does not document the `Privacy_*` category anchors as a supported API. They therefore need per-supported-OS acceptance coverage and a root fallback.

## Statically verified in this repository

| Area | Current state | Consequence |
|---|---|---|
| Deployment | `apps/macos/Package.swift` declares `.macOS(.v14)` | Compatibility claims must cover macOS 14+; this research has only a macOS 26 live host. |
| FDA | `PermissionsManager.refresh()` calls `isReadableFile` on Mail V10 and Notes SQLite | It is a metadata check, collapses two needs into one boolean, and does not prove that the daemon/helper that reads the data has access. Use actual open attempts and expose structural per-resource status. |
| Screen Recording | Correct preflight API; request result is ignored and immediately rechecked | Keep the preflight, consume the request result only as request feedback, and recheck on activation. |
| Accessibility | Correct trust APIs; prompt is followed by an immediate refresh | The immediate refresh races an asynchronous prompt. Recheck on activation; the paste-wheel event tap already retries every five seconds. |
| Microphone | UI design requires a microphone row, but `PermissionsManager` and the current permissions page have no microphone property or row | Add the AVFoundation status/request path and `NSMicrophoneUsageDescription` before exposing the row. Current bundle metadata has neither microphone usage text nor an audio-input entitlement. |
| Deep links | FDA, Screen Recording, and Accessibility use the anchors listed below; microphone has no code path | Centralize all four anchors plus the Privacy & Security root fallback. |
| Activation | No app-activation permission observer exists | Returning from System Settings does not refresh until another health refresh/app startup. |
| Relaunch | `AppDelegate` calls `DaemonLauncher.launch()` on every UI launch; `stop()` intentionally leaves the daemon alive | Simply reopening the app restarts the launchd daemon and is not safe for active automations. |
| UI restoration | Only `sessionId` survives in `UserDefaults`; `messages`, `inputText`, `notchSettingsPage`, and interaction mode initialize empty/general/collapsed | A UI-only relaunch needs a versioned, bounded handoff snapshot for current session ID, draft, transcript or refetch cursor, settings page, and permission-assist step. Do not persist evidence content beyond the existing product policy. |
| Packaging/signing | The Makefile bundle contains only two executables and `Info.plist`, then signs the outer app; there is no `Contents/Resources`, icon declaration, hardened-runtime option, or Developer ID/notarization path | Add resources before signing; sign nested executable code appropriately, then sign/verify the outer bundle. Development signing is stable on this host but is not a distribution pipeline. |
| Icon | No PNG, ICNS, asset catalog, or icon declaration exists in this worktree/current history | The signed installed app currently has a generic associated icon. The sibling-only PNG must remain untouched until deliberately imported/normalized by implementation work. |

Current source paths: [`PermissionsManager.swift`](../apps/macos/Sources/bagent/PermissionsManager.swift), [`NotchSettingsContent.swift`](../apps/macos/Sources/bagent/NotchSettingsContent.swift), [`AppDelegate.swift`](../apps/macos/Sources/bagent/AppDelegate.swift), [`DaemonLauncher.swift`](../apps/macos/Sources/bagent/DaemonLauncher.swift), [`Package.swift`](../apps/macos/Package.swift), and [`Makefile`](../apps/macos/Makefile).

## Live verified without mutation

On macOS 26.5.2, `NSWorkspace`/LaunchServices accepted each URL and the System Settings accessibility hierarchy exposed the expected destination title:

| Destination | URL | Observed title |
|---|---|---|
| Privacy & Security fallback | `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension` | Privacy & Security |
| Full Disk Access | `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles` | Full Disk Access |
| Screen Recording | `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture` | Screen & System Audio Recording |
| Accessibility | `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` | Accessibility |
| Microphone | `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` | Microphone |

Additional structural checks:

- `/Applications/bagent.app` was a valid `com.apple.application-bundle`, strict deep verification passed, and its signature reported identifier `sk.bagent.app`, Apple Development team `QUB47S3XTF`.
- Writing `NSURL(fileURLWithPath: "/Applications/bagent.app")` to a fresh drag pasteboard succeeded, advertised exactly `public.file-url`, and round-tripped to the same bundle path. This validates the payload shape without dropping it on a privacy pane.
- The installed bundle had no icon resource/declaration; `NSWorkspace.icon(forFile:)` returned the documented initial 32×32 representation, structurally consistent with a generic bundle icon.
- System Settings already listed the installed signed app for FDA, Screen Recording, and Accessibility. Microphone did not list it. These observations validate discoverability/current UI state only; no switch was touched and no prompt was invoked.
- The current FDA probe paths both existed on this host (`Mail/V10/.../Envelope Index` and the Notes SQLite store). No Mail or Notes content was read.

During observation, an unrelated pre-existing acceptance process stopped the app, daemon, and BaseRT and unregistered their launch agents. This research did not signal, restart, bootstrap, or otherwise alter those processes, and it left them stopped as found afterward.

## Unverified because mutation was prohibited

- Category-anchor behavior on macOS 14 and 15, localization variants, and any future macOS 26 update.
- Whether dragging the exact file-URL payload into each pane adds the installed signed bundle and what feedback System Settings shows.
- The transition timing of every probe after a fresh grant/revoke, including which OS versions display Quit & Reopen or Apply Later.
- Effective FDA inheritance/attribution for the launchd-owned `Contents/MacOS/bagentd` helper. This matters because the UI probes itself while Mail/Notes access occurs in a different process.
- End-to-end replacement-UI handoff while an Automation Run is active, including unchanged daemon/BaseRT PIDs and exact restoration of current chat, draft, and permission page.
- Display quality of a normalized/compiled version of `bagent_icon.png` in Finder, the drag image, and every supported System Settings pane.

## Acceptance gates for implementation tickets

1. Unit-test URL selection, probe state mapping (including microphone's four AV statuses), activation debounce, and bounded relaunch snapshot encoding without invoking prompt APIs.
2. Build a signed bundle with a stable designated requirement, declared icon resources, required usage strings, and separately valid nested code; pass `codesign --verify --deep --strict` and inspect the designated requirements.
3. On macOS 14, 15, and 26, visually verify every category link and root fallback, then drag the running installed `.app` and confirm the entry/icon without using source/debug binaries.
4. In a disposable TCC test account or VM, verify probe transitions and relaunch requirements one permission at a time. Never use `tccutil reset` on the developer's normal account.
5. Start a daemon-owned Automation Run, capture daemon and BaseRT PIDs, perform the UI-only handoff, and require unchanged PIDs, uninterrupted run completion, restored session/draft/permission step, and zero duplicate event subscriptions.
6. Verify the ordinary cold-launch/upgrade path remains distinct: only that path may reinstall/restart the daemon when the packaged daemon identity or configuration actually changed.
