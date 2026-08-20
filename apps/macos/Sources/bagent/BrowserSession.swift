import AppKit
import Combine
import Foundation
import WebKit

@MainActor
final class BrowserSession: NSObject, WKNavigationDelegate, WKUIDelegate {
    let id: UUID
    let profile: BrowserProfile
    let navigationPolicy: BrowserNavigationPolicy
    let ownerLabel: String

    private(set) var ownerConnectionID: String
    private(set) var stateMachine = BrowserSessionStateMachine()
    private(set) var pageRevision = 0
    private(set) var webView: WKWebView
    private(set) var bridge: BrowserPageBridge
    let windowController: BrowserWindowController

    private var navigationWaiters: [UUID: CheckedContinuation<Void, Error>] = [:]
    private var pendingAlert: (() -> Void)?
    private var alertMessage: String?
    private var submissionGrants: Set<String> = []
    private var lastURL: URL?
    private var processTerminated = false

    var onChange: (() -> Void)?
    var onManualPreemption: (() -> Void)?
    var onSubmissionRequest: (() -> Void)?

    init(
        id: UUID = UUID(),
        ownerConnectionID: String,
        ownerLabel: String,
        profile: BrowserProfile,
        navigationPolicy: BrowserNavigationPolicy = BrowserNavigationPolicy()
    ) {
        self.id = id
        self.ownerConnectionID = ownerConnectionID
        self.ownerLabel = ownerLabel
        self.profile = profile
        self.navigationPolicy = navigationPolicy

        let webView = Self.makeWebView(profile: profile)
        self.webView = webView
        self.bridge = BrowserPageBridge(webView: webView)
        self.windowController = BrowserWindowController(webView: webView)
        super.init()

        configure(webView: webView)
        windowController.onManualInput = { [weak self] in
            self?.manualInput()
        }
        windowController.onManualResize = { [weak self] in
            self?.manualInput()
        }
        windowController.onCollapseRequested = { [weak self] in
            self?.hide()
        }
        try? stateMachine.ready()
    }

    var cue: BrowserCue {
        BrowserCue(id: id, label: ownerLabel, state: cueState, origin: currentOrigin,
                   isAgentActive: isAgentActive)
    }

    /// How long after an MCP call the session still counts as actively driven
    /// by its agent.
    // ponytail: fixed window; make it per-tool if long-running calls need it.
    static let agentActivityWindow: TimeInterval = 4

    private(set) var lastAgentActivity: Date?

    /// True while Codex/Claude is actually using this Browser Session.
    var isAgentActive: Bool {
        guard stateMachine.ownership == .connected, let lastAgentActivity else { return false }
        return Date().timeIntervalSince(lastAgentActivity) < Self.agentActivityWindow
    }

    func markAgentActivity(at date: Date = Date()) {
        lastAgentActivity = date
        onChange?()
    }

    var currentOrigin: String? {
        guard let url = webView.url else { return nil }
        return navigationPolicy.origin(for: url)
    }

    var pageInfo: BrowserPageInfo {
        BrowserPageInfo(
            url: redactedURL(webView.url),
            origin: currentOrigin,
            title: webView.title ?? "",
            loadState: stateMachine.page,
            viewport: viewport,
            visibility: stateMachine.visibility,
            revision: pageRevision,
            ownership: stateMachine.ownership,
            control: stateMachine.control
        )
    }

    var viewport: BrowserViewport {
        let bounds = webView.bounds
        let scale = webView.window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 1
        return BrowserViewport(width: max(0, Int(bounds.width)), height: max(0, Int(bounds.height)), backingScale: scale)
    }

    var isDetached: Bool { stateMachine.ownership == .detached }

    func navigate(to url: URL) async throws {
        guard !processTerminated else {
            throw BrowserFailure(.browserProcessTerminated, "The WebKit content process terminated. The page was not replayed.")
        }
        switch navigationPolicy.validate(url) {
        case .success:
            break
        case .failure(let error):
            throw error
        }
        let waiterID = UUID()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            navigationWaiters[waiterID] = continuation
            scheduleNavigationTimeout(waiterID)
            lastURL = url
            webView.load(URLRequest(url: url))
        }
    }

    func waitForNavigation(timeoutMilliseconds: Int = 15_000) async throws {
        guard stateMachine.page == .loading else { return }
        let waiterID = UUID()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            navigationWaiters[waiterID] = continuation
            scheduleNavigationTimeout(waiterID, milliseconds: timeoutMilliseconds)
        }
    }

    func snapshot(maxCharacters: Int = 20_000) async throws -> BrowserPageSnapshot {
        guard !processTerminated else {
            throw BrowserFailure(.browserProcessTerminated, "The WebKit content process terminated. The page snapshot is invalid.")
        }
        return try await bridge.snapshot(revision: pageRevision, maxCharacters: maxCharacters)
    }

    func screenshot(region: String, displayID: CGDirectDisplayID? = nil) async throws -> String {
        guard !processTerminated else {
            throw BrowserFailure(.browserProcessTerminated, "The WebKit content process terminated. The screenshot is invalid.")
        }
        let originalSize = webView.bounds.size
        let captureSize: CGSize
        switch region {
        case "viewport":
            captureSize = originalSize
        case "screen":
            captureSize = windowController.display(for: displayID)?.visibleFrame.size ?? originalSize
        default:
            throw BrowserFailure(.invalidRequest, "screenshot region must be viewport or screen.")
        }

        if captureSize != originalSize {
            applyViewport(width: Int(captureSize.width), height: Int(captureSize.height))
        }
        defer {
            if captureSize != originalSize {
                applyViewport(width: Int(originalSize.width), height: Int(originalSize.height))
            }
        }
        windowController.panel.layoutIfNeeded()
        await Task.yield()

        let configuration = WKSnapshotConfiguration()
        configuration.rect = webView.bounds
        configuration.afterScreenUpdates = true
        let image = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<NSImage, Error>) in
            webView.takeSnapshot(with: configuration) { image, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let image {
                    continuation.resume(returning: image)
                } else {
                    continuation.resume(throwing: BrowserFailure(.operationTimedOut, "WebKit returned no screenshot image."))
                }
            }
        }
        guard let tiff = image.tiffRepresentation,
              let bitmap = NSBitmapImageRep(data: tiff),
              let png = bitmap.representation(using: .png, properties: [:]) else {
            throw BrowserFailure(.operationTimedOut, "The screenshot could not be encoded as PNG.")
        }
        return png.base64EncodedString()
    }

    func setViewport(width: Int, height: Int) throws {
        guard stateMachine.ownership == .connected, stateMachine.control == .agent else {
            throw BrowserFailure(.controlRevokedByUser, "The user controls this Browser Session.")
        }
        applyViewport(width: width, height: height)
    }

    private func applyViewport(width: Int, height: Int) {
        let safeWidth = min(max(width, 320), 3_840)
        let safeHeight = min(max(height, 240), 2_160)
        windowController.setContentSize(NSSize(width: safeWidth, height: safeHeight))
        webView.frame = CGRect(x: 0, y: 0, width: safeWidth, height: safeHeight)
        onChange?()
    }

    private func coordinateInteraction(_ action: BrowserAction) throws -> BrowserInteractionResult {
        guard stateMachine.visibility == .popup, windowController.panel.isKeyWindow else {
            markAttention()
            throw BrowserFailure(.visibleInteractionRequired, "Coordinate interaction requires the visible, key Browser Panel.")
        }
        guard let x = action.x, let y = action.y,
              x >= 0, y >= 0, x <= webView.bounds.width, y <= webView.bounds.height else {
            throw BrowserFailure(.invalidRequest, "Coordinate interaction is outside the current viewport.")
        }
        let viewPoint = CGPoint(x: x, y: webView.bounds.height - y)
        let windowPoint = webView.convert(viewPoint, to: nil)
        let timestamp = ProcessInfo.processInfo.systemUptime
        switch action.type {
        case "click":
            guard let down = NSEvent.mouseEvent(
                with: .leftMouseDown, location: windowPoint, modifierFlags: [], timestamp: timestamp,
                windowNumber: windowController.panel.windowNumber, context: nil, eventNumber: 0,
                clickCount: 1, pressure: 1
            ), let up = NSEvent.mouseEvent(
                with: .leftMouseUp, location: windowPoint, modifierFlags: [], timestamp: timestamp,
                windowNumber: windowController.panel.windowNumber, context: nil, eventNumber: 0,
                clickCount: 1, pressure: 0
            ) else {
                throw BrowserFailure(.visibleInteractionRequired, "The Browser Panel could not create a native mouse event.")
            }
            webView.mouseDown(with: down)
            webView.mouseUp(with: up)
        case "hover", "move":
            guard let event = NSEvent.mouseEvent(
                with: .mouseMoved, location: windowPoint, modifierFlags: [], timestamp: timestamp,
                windowNumber: windowController.panel.windowNumber, context: nil, eventNumber: 0,
                clickCount: 0, pressure: 0
            ) else {
                throw BrowserFailure(.visibleInteractionRequired, "The Browser Panel could not create a native mouse event.")
            }
            webView.mouseMoved(with: event)
        case "scroll":
            guard let scrollView = webView.enclosingScrollView else {
                throw BrowserFailure(.visibleInteractionRequired, "The Browser Panel has no native scroll container.")
            }
            let clipView = scrollView.contentView
            let current = clipView.bounds.origin
            clipView.scroll(to: CGPoint(
                x: current.x - (action.deltaX ?? 0),
                y: current.y - (action.deltaY ?? 0)
            ))
            scrollView.reflectScrolledClipView(clipView)
        default:
            throw BrowserFailure(.visibleInteractionRequired, "This coordinate action requires a supported native input path.")
        }
        onChange?()
        return BrowserInteractionResult(
            action: action.type,
            method: "coordinate",
            finalURL: redactedURL(webView.url),
            revision: pageRevision,
            navigationBegan: false,
            control: stateMachine.control
        )
    }

    func show(focus: Bool) throws {
        guard stateMachine.ownership == .connected else {
            throw BrowserFailure(.connectionNotOwner, "A Detached Session must be reclaimed by the user before it can be shown to an agent.")
        }
        try stateMachine.setVisibility(.popup)
        windowController.show(focus: focus)
        if let origin = currentOrigin { windowController.showOrigin(origin) }
        onChange?()
    }

    func hide() {
        windowController.hide()
        try? stateMachine.setVisibility(.hidden)
        if stateMachine.control == .user {
            try? stateMachine.resumeAgent()
        }
        onChange?()
    }

    func detach() {
        try? stateMachine.detach()
        onChange?()
    }

    func requestReclaim() throws {
        try stateMachine.requestReclaim()
        onChange?()
    }

    func reclaim(to connectionID: String, ownerLabel: String) throws {
        try stateMachine.reclaim()
        ownerConnectionID = connectionID
        onChange?()
    }

    func cancelReclaim() {
        try? stateMachine.cancelReclaim()
        onChange?()
    }

    func grantSubmission(for origin: String) {
        submissionGrants.insert(origin)
    }

    func interact(_ action: BrowserAction) async throws -> BrowserInteractionResult {
        guard stateMachine.ownership == .connected, stateMachine.control == .agent else {
            throw BrowserFailure(.controlRevokedByUser, "The user controls this Browser Session.")
        }
        let visibilityAtStart = stateMachine.visibility
        defer {
            // Semantic WebKit work must not make a hidden panel enter the
            // window list. Preserve an explicit popup or user reveal if one
            // happened while the async page operation was in flight.
            if visibilityAtStart == .hidden,
               stateMachine.visibility == .hidden,
               windowController.panel.isVisible {
                windowController.hide()
            }
        }
        switch action.type {
        case "camera", "microphone", "screen_share", "location", "geolocation", "notifications", "authentication", "hardware_auth", "download", "upload", "clipboard", "drag_drop", "evaluate_javascript", "javascript":
            throw BrowserFailure(.permissionNotSupported, "This browser capability is not supported by bagent Browser.")
        case "popup", "open_window":
            throw BrowserFailure(.popupNotSupported, "Popup windows are not supported by bagent Browser.")
        default:
            break
        }
        guard action.reference == nil || action.revision == pageRevision else {
            throw BrowserFailure(.staleElementReference, "The Element Reference belongs to an older Page Snapshot revision.")
        }

        if action.reference == nil, action.x != nil, action.y != nil {
            return try coordinateInteraction(action)
        }

        var evaluation = try await bridge.perform(action, allowSubmission: false)
        if evaluation.passwordField {
            markAttention()
            throw BrowserFailure(.passwordFieldForbidden, "Password fields are human-only and require visible interaction.")
        }
        if evaluation.nativeRequired {
            markAttention()
            throw BrowserFailure(.visibleInteractionRequired, "This control requires direct input in the visible Browser Panel.")
        }
        if evaluation.submissionRequired {
            guard let origin = currentOrigin, submissionGrants.contains(origin) else {
                markAttention()
                onSubmissionRequest?()
                throw BrowserFailure(.submissionGrantRequired, "User approval is required for every form submission on this Browser Session and origin. Destructive submissions are included.", details: ["origin": currentOrigin ?? "unknown"])
            }
            evaluation = try await bridge.perform(action, allowSubmission: true)
        }
        guard evaluation.ok else {
            throw BrowserFailure(.invalidRequest, evaluation.error ?? "The page rejected the semantic action.")
        }
        onChange?()
        return BrowserInteractionResult(
            action: action.type,
            method: "dom",
            finalURL: redactedURL(webView.url),
            revision: pageRevision,
            navigationBegan: evaluation.navigationBegan,
            control: stateMachine.control
        )
    }

    func acknowledgeAlert() throws {
        guard let pendingAlert else {
            throw BrowserFailure(.invalidRequest, "There is no plain alert waiting for acknowledgement.")
        }
        pendingAlert()
        self.pendingAlert = nil
        alertMessage = nil
        onChange?()
    }

    func alertInfo() -> String? { alertMessage }

    func manualInput() {
        guard stateMachine.control == .agent else { return }
        stateMachine.revokeControl()
        navigationWaiters.values.forEach { $0.resume(throwing: BrowserFailure(.controlRevokedByUser, "Direct user input revoked the Control Lease.")) }
        navigationWaiters.removeAll()
        onManualPreemption?()
        onChange?()
    }

    func releaseControl() {
        if stateMachine.control == .agent { stateMachine.revokeControl() }
        onChange?()
    }

    func requireUserAttention() {
        markAttention()
    }

    func resumeAgent() throws {
        try stateMachine.resumeAgent()
        onChange?()
    }

    func consoleMessages() -> [BrowserConsoleMessage] { bridge.consoleMessages }
    func networkRequests() -> [BrowserNetworkRequest] { bridge.networkRequests }

    func recoverAfterProcessTermination() {
        guard processTerminated else { return }
        let replacement = Self.makeWebView(profile: profile)
        webView = replacement
        bridge = BrowserPageBridge(webView: replacement)
        configure(webView: replacement)
        windowController.replaceWebView(replacement)
        processTerminated = false
        pageRevision += 1
        stateMachine.failPage()
        onChange?()
    }

    func terminate() {
        stateMachine.terminate()
        windowController.hide()
        navigationWaiters.values.forEach { $0.resume(throwing: BrowserFailure(.browserProcessTerminated, "bagent Browser is terminating.")) }
        navigationWaiters.removeAll()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
        onChange?()
    }

    private var cueState: BrowserCueState {
        switch stateMachine.ownership {
        case .detached: return .detached
        case .reclaimPending: return .reclaimPending
        case .connected:
            switch stateMachine.control {
            case .waitingForUser, .user: return .attention
            case .agent: return stateMachine.page == .loading ? .active : .steady
            }
        }
    }

    private func markAttention() {
        stateMachine.revokeControl()
        onChange?()
    }

    private func configure(webView: WKWebView) {
        webView.navigationDelegate = self
        webView.uiDelegate = self
    }

    private static func makeWebView(profile: BrowserProfile) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = profile.dataStore
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        configuration.suppressesIncrementalRendering = false
        let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 960, height: 640), configuration: configuration)
        // Explicit, not just relying on the platform default — SECURITY.md
        // names this as a release-build requirement, not an incidental gap.
        #if DEBUG
        webView.isInspectable = true
        #else
        webView.isInspectable = false
        #endif
        return webView
    }

    private func redactedURL(_ url: URL?) -> String? {
        guard let url, var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        components.query = nil
        components.fragment = nil
        return components.string
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
        if navigationAction.shouldPerformDownload {
            decisionHandler(.cancel)
            return
        }
        guard navigationAction.targetFrame?.isMainFrame != false else {
            decisionHandler(.cancel)
            return
        }
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        switch navigationPolicy.validate(url) {
        case .success: decisionHandler(.allow)
        case .failure: decisionHandler(.cancel)
        }
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse, decisionHandler: @escaping @MainActor @Sendable (WKNavigationResponsePolicy) -> Void) {
        if !navigationResponse.canShowMIMEType {
            decisionHandler(.cancel)
            return
        }
        guard let url = navigationResponse.response.url else {
            decisionHandler(.cancel)
            return
        }
        switch navigationPolicy.validate(url) {
        case .success: decisionHandler(.allow)
        case .failure: decisionHandler(.cancel)
        }
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        pageRevision += 1
        bridge.invalidateReferences(revision: pageRevision)
        try? stateMachine.beginLoading()
        onChange?()
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        try? stateMachine.interactive()
        if let origin = currentOrigin { windowController.showOrigin(origin) }
        resumeNavigationWaiters()
        onChange?()
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        stateMachine.failPage()
        failNavigationWaiters(error: BrowserFailure(.navigationBlocked, "Navigation failed."))
        onChange?()
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        stateMachine.failPage()
        failNavigationWaiters(error: BrowserFailure(.navigationBlocked, "Navigation failed before the page loaded."))
        onChange?()
    }

    func webView(_ webView: WKWebView, didReceiveServerRedirectForProvisionalNavigation navigation: WKNavigation!) {
        guard let url = webView.url, case .failure = navigationPolicy.validate(url) else { return }
        webView.stopLoading()
        failNavigationWaiters(error: BrowserFailure(.navigationBlocked, "Redirect destination is outside the Navigation Allowlist."))
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        processTerminated = true
        pageRevision += 1
        bridge.invalidateReferences(revision: pageRevision)
        stateMachine.failPage()
        failNavigationWaiters(error: BrowserFailure(.browserProcessTerminated, "The WebKit content process terminated. No mutation was replayed."))
        onChange?()
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        nil
    }

    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor @Sendable (WKPermissionDecision) -> Void
    ) {
        decisionHandler(.deny)
        onChange?()
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.cancel()
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.cancel()
    }

    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor @Sendable () -> Void) {
        alertMessage = String(message.prefix(2_000))
        pendingAlert = completionHandler
        onChange?()
    }

    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor @Sendable (Bool) -> Void) {
        completionHandler(false)
        markAttention()
    }

    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor @Sendable (String?) -> Void) {
        completionHandler(nil)
        markAttention()
    }

    private func resumeNavigationWaiters() {
        navigationWaiters.values.forEach { $0.resume() }
        navigationWaiters.removeAll()
    }

    private func failNavigationWaiters(error: Error) {
        navigationWaiters.values.forEach { $0.resume(throwing: error) }
        navigationWaiters.removeAll()
    }

    private func scheduleNavigationTimeout(_ waiterID: UUID, milliseconds: Int = 15_000) {
        let boundedMilliseconds = min(max(milliseconds, 100), 50_000)
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(boundedMilliseconds))
            guard let self, let waiter = self.navigationWaiters.removeValue(forKey: waiterID) else { return }
            waiter.resume(throwing: BrowserFailure(.operationTimedOut, "Navigation did not settle before the browser deadline."))
        }
    }
}
