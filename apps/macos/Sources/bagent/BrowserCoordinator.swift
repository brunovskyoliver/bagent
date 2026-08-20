import Combine
import Foundation
import WebKit

@MainActor
final class BrowserCoordinator: NSObject, ObservableObject {
    static let browserEnabledKey = "browserEnabled"

    @Published private(set) var cues: [BrowserCue] = []
    @Published private(set) var isEnabled: Bool

    let profile: BrowserProfile
    let auditLog: BrowserAuditLog
    let navigationPolicy: BrowserNavigationPolicy

    private(set) var sessions: [UUID: BrowserSession] = [:]
    private var registry = BrowserSessionRegistry(limit: 4)
    private var pendingCloseSessionID: UUID?
    private var reclaimRequests: [UUID: (connectionID: String, label: String)] = [:]
    private var submissionRequests: Set<UUID> = []
    private var activeCueDragSessionID: UUID?
    /// Injected by the notch window controller — the preview renders inside
    /// its panel's own content view rather than a floating window of its own.
    weak var cuePreviewHost: BrowserCuePreviewHosting?
    private var previewSessionID: UUID?
    private var activityExpiryTasks: [UUID: Task<Void, Never>] = [:]
    private var cueScreenRects: [UUID: CGRect] = [:]
    /// Bumped per cue to play the "this session is busy" shake.
    @Published private(set) var cueShakes: [UUID: Int] = [:]
    /// Screen rect of the notch's collapsed hit region — dropping a Browser
    /// Panel here collapses it back into its cue.
    var notchDropZoneProvider: (() -> CGRect)?
    /// Height of the panel's leading edge that counts as "over the notch".
    static let panelDropBandHeight: CGFloat = 44
    /// True while a dragged panel is over the notch, so the notch can react.
    @Published private(set) var isNotchDropTargeted = false
    var onEnabledChanged: ((Bool) -> Void)?

    init(
        profile: BrowserProfile = BrowserProfile(),
        navigationPolicy: BrowserNavigationPolicy = BrowserNavigationPolicy(),
        enabled: Bool? = nil
    ) {
        self.profile = profile
        self.navigationPolicy = navigationPolicy
        self.auditLog = BrowserAuditLog()
        let environmentEnabled = ProcessInfo.processInfo.environment["BAGENT_BROWSER_ENABLED"] == "1"
        self.isEnabled = enabled ?? (environmentEnabled || UserDefaults.standard.bool(forKey: Self.browserEnabledKey))
        super.init()
    }

    func setEnabled(_ enabled: Bool) {
        guard isEnabled != enabled else { return }
        UserDefaults.standard.set(enabled, forKey: Self.browserEnabledKey)
        isEnabled = enabled
        if !enabled {
            for session in sessions.values { session.terminate() }
            sessions.removeAll()
            registry = BrowserSessionRegistry(limit: 4)
            pendingCloseSessionID = nil
            reclaimRequests.removeAll()
            submissionRequests.removeAll()
            activeCueDragSessionID = nil
            refreshCues()
        }
        onEnabledChanged?(enabled)
    }

    func start() {
        // The browser profile and socket are lazy. No WebKit view is created here.
    }

    func handle(connectionID: String, connectionLabel: String, request: BrowserRPCRequest) async -> BrowserRPCResponse {
        defer { markAgentActivity(for: connectionID) }
        do {
            guard isEnabled else {
                throw BrowserFailure(
                    .browserDisabled,
                    "bagent Browser is disabled. Enable bagent Browser in Settings, then retry."
                )
            }
            let result = try await dispatch(connectionID: connectionID, connectionLabel: connectionLabel, request: request)
            auditLog.record(connectionLabel: connectionLabel, tool: request.method, origin: origin(for: connectionID), resultClass: "success")
            return .success(id: request.id, result: result)
        } catch let failure as BrowserFailure {
            auditLog.record(connectionLabel: connectionLabel, tool: request.method, origin: origin(for: connectionID), resultClass: failure.code.rawValue)
            return .failure(id: request.id, failure)
        } catch {
            let failure = BrowserFailure(.invalidRequest, "The browser command failed.")
            auditLog.record(connectionLabel: connectionLabel, tool: request.method, origin: origin(for: connectionID), resultClass: failure.code.rawValue)
            return .failure(id: request.id, failure)
        }
    }

    func connectionDidClose(_ connectionID: String) {
        let canceledReclaims = reclaimRequests.filter { $0.value.connectionID == connectionID }.map(\.key)
        for sessionID in canceledReclaims {
            sessions[sessionID]?.cancelReclaim()
        }
        reclaimRequests = reclaimRequests.filter { $0.value.connectionID != connectionID }
        guard let sessionID = registry.session(for: connectionID), let session = sessions[sessionID] else { return }
        registry.release(connectionID: connectionID)
        submissionRequests.remove(sessionID)
        if activeCueDragSessionID == sessionID { activeCueDragSessionID = nil }
        session.detach()
        refreshCues()
    }

    /// Every MCP call marks its session as actively driven, and schedules the
    /// refresh that clears the mark once the window lapses.
    private func markAgentActivity(for connectionID: String) {
        guard let sessionID = registry.session(for: connectionID),
              let session = sessions[sessionID] else { return }
        session.markAgentActivity()
        refreshCues()
        activityExpiryTasks[sessionID]?.cancel()
        activityExpiryTasks[sessionID] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(BrowserSession.agentActivityWindow))
            guard !Task.isCancelled else { return }
            self?.refreshCues()
        }
    }

    /// ⌥-click on a Browser Cue destroys that session — unless its agent is
    /// mid-session, in which case the cue shakes instead.
    func optionClickCue(_ sessionID: UUID) {
        guard let session = sessions[sessionID] else { return }
        if session.isAgentActive {
            cueShakes[sessionID, default: 0] += 1
            return
        }
        hideCuePreview()
        approveClose(sessionID: sessionID)
    }

    func toggleCue(_ sessionID: UUID) {
        guard let session = sessions[sessionID] else { return }
        if session.pageInfo.visibility == .popup {
            session.hide()
        } else {
            session.windowController.nextReveal = .fromCue
            try? session.show(focus: true)
        }
        refreshCues()
    }

    /// Where this session's Browser Cue currently sits on screen, so the panel
    /// can grow out of it and collapse back into it.
    func updateCueRect(_ sessionID: UUID, _ rect: CGRect) {
        sessions[sessionID]?.windowController.cueScreenRect = rect
        cueScreenRects[sessionID] = rect
    }

    /// Whether a screen point lands on a Browser Cue. Padded, because the busy
    /// shake moves the drawn icon a few points away from its view's frame.
    func isPointOverCue(_ point: CGPoint) -> Bool {
        cueScreenRects.values.contains { $0.insetBy(dx: -5, dy: -5).contains(point) }
    }

    /// Dragging the Browser Panel by its top strip. Dropping it on the notch
    /// collapses the panel into its cue; the session stays alive.
    func panelDrag(_ sessionID: UUID, phase: BrowserCueDragPhase, at pointer: CGPoint) {
        guard let session = sessions[sessionID] else { return }
        let zone = notchDropZoneProvider?() ?? .zero
        // Either the cursor is in the notch, or the panel's own top edge has
        // been pushed into it — dragging by the top strip keeps the cursor well
        // below the menu bar, so the pointer alone missed almost every drop.
        // The panel's *projected* frame: where it would sit if it were still
        // following the cursor. Using its real frame would latch the drop state
        // on, because a previewed collapse parks it inside the notch.
        let panelFrame = session.windowController.projectedDragFrame(at: pointer)
        let panelTopBand = CGRect(
            x: panelFrame.minX,
            y: panelFrame.maxY - Self.panelDropBandHeight,
            width: panelFrame.width,
            height: Self.panelDropBandHeight
        )
        let overNotch = zone.contains(pointer) || zone.intersects(panelTopBand)
        BrowserCueTrace.log(
            "panelDrag \(phase) pointer=\(pointer) zone=\(zone) panelTop=\(panelTopBand) over=\(overNotch)"
        )
        switch phase {
        case .began:
            session.windowController.beginPanelDrag(at: pointer)
            isNotchDropTargeted = overNotch
        case .changed:
            if isNotchDropTargeted != overNotch { isNotchDropTargeted = overNotch }
            // Entering the notch previews the collapse; leaving springs the
            // panel back to full size under the cursor and the drag continues.
            session.windowController.setDropPreview(overNotch, notchRect: zone, pointer: pointer)
        case .ended:
            isNotchDropTargeted = false
            if overNotch {
                // Release confirms the previewed collapse.
                session.hide()
                refreshCues()
            } else {
                session.windowController.setDropPreview(false, notchRect: zone, pointer: pointer)
            }
        }
    }

    /// Hover preview: which tab this cue is, shown under the cursor so the user
    /// knows what they are about to drag.
    func previewCue(_ sessionID: UUID, at cursor: CGPoint) {
        guard let session = sessions[sessionID] else { return }
        if previewSessionID == sessionID {
            cuePreviewHost?.moveCuePreview(to: cursor)
            return
        }
        previewSessionID = sessionID
        cuePreviewHost?.showCuePreview(title: session.pageInfo.title, origin: session.currentOrigin, at: cursor)

        let configuration = WKSnapshotConfiguration()
        configuration.snapshotWidth = NSNumber(value: Int(BrowserCuePreviewGeometry.size.width))
        session.webView.takeSnapshot(with: configuration) { [weak self] image, _ in
            guard let self, self.previewSessionID == sessionID else { return }
            self.cuePreviewHost?.setCuePreviewThumbnail(image)
        }
    }

    func hideCuePreview() {
        previewSessionID = nil
        cuePreviewHost?.hideCuePreview()
    }

    var isCuePreviewVisible: Bool { previewSessionID != nil }

    func dragCue(_ sessionID: UUID, phase: BrowserCueDragPhase, location: CGPoint) {
        // The panel itself takes over from the preview as soon as a drag starts.
        hideCuePreview()
        BrowserCueTrace.log("coordinator \(phase) session=\(sessionID) known=\(sessions[sessionID] != nil)")
        guard let session = sessions[sessionID] else { return }

        switch phase {
        case .began:
            guard activeCueDragSessionID == nil || activeCueDragSessionID == sessionID else { return }
            do {
                session.windowController.nextReveal = .inPlace
                // Never focus on the drag path: the Browser Panel is not a
                // nonactivating panel, so makeKeyAndOrderFront would activate
                // bagent and swap the key window in the middle of the cue's
                // mouse-tracking loop.
                try session.show(focus: false)
            } catch {
                BrowserCueTrace.log("coordinator began show failed: \(error)")
                return
            }
            activeCueDragSessionID = sessionID
            session.windowController.setFrameForCueDrag(at: location)
            BrowserCueTrace.log(
                "coordinator began shown visible=\(session.windowController.panel.isVisible) "
                + "frame=\(session.windowController.panel.frame) "
                + "hidesOnDeactivate=\(session.windowController.panel.hidesOnDeactivate)"
            )
        case .changed:
            guard activeCueDragSessionID == sessionID else { return }
            session.windowController.setFrameForCueDrag(at: location)
        case .ended:
            guard activeCueDragSessionID == sessionID else { return }
            session.windowController.setFrameForCueDrag(at: location)
            activeCueDragSessionID = nil
        }
    }

    func grantSubmission(sessionID: UUID, origin: String) {
        sessions[sessionID]?.grantSubmission(for: origin)
    }

    func approveSubmission(sessionID: UUID) {
        guard let session = sessions[sessionID], let origin = session.currentOrigin else { return }
        session.grantSubmission(for: origin)
        submissionRequests.remove(sessionID)
        try? session.resumeAgent()
        refreshCues()
    }

    func resumeAgent(sessionID: UUID) {
        try? sessions[sessionID]?.resumeAgent()
        refreshCues()
    }

    func isSubmissionPending(for sessionID: UUID) -> Bool {
        submissionRequests.contains(sessionID)
    }

    func isClosePending(for sessionID: UUID) -> Bool {
        pendingCloseSessionID == sessionID
    }

    func reclaimLabel(for sessionID: UUID) -> String? {
        reclaimRequests[sessionID]?.label
    }

    func approveReclaim(sessionID: UUID, connectionID: String, label: String) throws {
        guard let session = sessions[sessionID] else {
            throw BrowserFailure(.sessionNotFound, "The Detached Session no longer exists.")
        }
        try registry.claim(connectionID: connectionID, sessionID: sessionID)
        try session.reclaim(to: connectionID, ownerLabel: label)
        refreshCues()
    }

    func approvePendingReclaim(sessionID: UUID) throws {
        guard let request = reclaimRequests[sessionID] else {
            throw BrowserFailure(.sessionNotFound, "There is no pending reclaim request for this Detached Session.")
        }
        try approveReclaim(sessionID: sessionID, connectionID: request.connectionID, label: request.label)
        reclaimRequests.removeValue(forKey: sessionID)
    }

    func rejectPendingReclaim(sessionID: UUID) {
        guard reclaimRequests.removeValue(forKey: sessionID) != nil else { return }
        sessions[sessionID]?.cancelReclaim()
        refreshCues()
    }

    func approveClose(sessionID: UUID) {
        guard let session = sessions.removeValue(forKey: sessionID) else { return }
        activityExpiryTasks.removeValue(forKey: sessionID)?.cancel()
        cueShakes.removeValue(forKey: sessionID)
        cueScreenRects.removeValue(forKey: sessionID)
        registry.release(connectionID: session.ownerConnectionID)
        session.terminate()
        pendingCloseSessionID = nil
        submissionRequests.remove(sessionID)
        reclaimRequests.removeValue(forKey: sessionID)
        if activeCueDragSessionID == sessionID { activeCueDragSessionID = nil }
        if previewSessionID == sessionID { hideCuePreview() }
        refreshCues()
    }

    func clearProfileAfterUserConfirmation() async {
        for session in sessions.values { session.terminate() }
        sessions.removeAll()
        registry = BrowserSessionRegistry(limit: 4)
        pendingCloseSessionID = nil
        reclaimRequests.removeAll()
        submissionRequests.removeAll()
        activeCueDragSessionID = nil
        refreshCues()
        await profile.clear()
    }

    private func dispatch(connectionID: String, connectionLabel: String, request: BrowserRPCRequest) async throws -> BrowserJSONValue {
        if request.method == "browser_open" {
            return try await open(connectionID: connectionID, connectionLabel: connectionLabel, params: request.params)
        }
        if request.method == "browser_request_reclaim" {
            return try reclaimRequest(connectionID: connectionID, connectionLabel: connectionLabel)
        }
        let session = try ownedSession(connectionID: connectionID)

        switch request.method {
        case "page_info":
            return try json(session.pageInfo)
        case "navigate_to_url":
            guard let rawURL = request.params["url"]?.stringValue, let url = URL(string: rawURL) else {
                throw BrowserFailure(.invalidRequest, "navigate_to_url requires a URL.")
            }
            try await session.navigate(to: url)
            return try json(session.pageInfo)
        case "get_page_content":
            let maxCharacters = request.params["max_chars"]?.intValue ?? 20_000
            return try json(await session.snapshot(maxCharacters: maxCharacters))
        case "screenshot":
            let region = request.params["region"]?.stringValue ?? "viewport"
            let displayID = request.params["display_id"]?.intValue.map(CGDirectDisplayID.init)
            let base64 = try await session.screenshot(region: region, displayID: displayID)
            return .object([
                "content_type": .string("image/png"),
                "data_base64": .string(base64),
                "page": try json(session.pageInfo),
            ])
        case "set_viewport_size":
            guard let width = request.params["width"]?.intValue, let height = request.params["height"]?.intValue else {
                throw BrowserFailure(.invalidRequest, "set_viewport_size requires width and height.")
            }
            try session.setViewport(width: width, height: height)
            return try json(session.pageInfo)
        case "browser_set_visibility":
            let visibility = request.params["visibility"]?.stringValue ?? "hidden"
            if visibility == "hidden" {
                session.hide()
            } else if visibility == "popup" {
                try session.show(focus: request.params["focus"]?.boolValue == true)
            } else {
                throw BrowserFailure(.invalidRequest, "visibility must be hidden or popup.")
            }
            return try json(session.pageInfo)
        case "browser_console_messages":
            return .object(["coverage": .string("partial"), "messages": try json(session.consoleMessages())])
        case "list_network_requests":
            return .object(["coverage": .string("partial"), "requests": try json(session.networkRequests())])
        case "page_interactions":
            return try await interactions(session: session, params: request.params)
        case "wait_for_navigation":
            try await session.waitForNavigation(timeoutMilliseconds: request.params["timeout_ms"]?.intValue ?? 15_000)
            return try json(session.pageInfo)
        case "browser_wait":
            return try await wait(session: session, params: request.params)
        case "browser_acknowledge_alert":
            try session.acknowledgeAlert()
            return try json(session.pageInfo)
        case "browser_release_control":
            session.releaseControl()
            return try json(session.pageInfo)
        case "browser_request_close":
            pendingCloseSessionID = session.id
            session.requireUserAttention()
            refreshCues()
            throw BrowserFailure(.approvalRequired, "User approval is required from the Browser Cue before closing this live page.")
        default:
            throw BrowserFailure(.invalidRequest, "Unknown browser tool.")
        }
    }

    private func open(connectionID: String, connectionLabel: String, params: [String: BrowserJSONValue]) async throws -> BrowserJSONValue {
        if let sessionID = registry.session(for: connectionID), let session = sessions[sessionID] {
            return try json(session.pageInfo)
        }
        let sessionID = UUID()
        try registry.claim(connectionID: connectionID, sessionID: sessionID)
        let session = BrowserSession(
            id: sessionID,
            ownerConnectionID: connectionID,
            ownerLabel: connectionLabel,
            profile: profile,
            navigationPolicy: navigationPolicy
        )
        session.onChange = { [weak self, weak session] in
            guard let self, session != nil else { return }
            self.refreshCues()
        }
        session.onManualPreemption = { [weak self] in self?.refreshCues() }
        session.windowController.onDragPhase = { [weak self] phase, pointer in
            self?.panelDrag(sessionID, phase: phase, at: pointer)
        }
        session.onSubmissionRequest = { [weak self] in
            self?.submissionRequests.insert(sessionID)
            self?.refreshCues()
        }
        sessions[sessionID] = session
        refreshCues()

        if let viewport = params["viewport"]?.objectValue,
           let width = viewport["width"]?.intValue,
           let height = viewport["height"]?.intValue {
            try session.setViewport(width: width, height: height)
        }
        if params["visible"]?.boolValue == true {
            throw BrowserFailure(.invalidRequest, "browser_open always creates a hidden Browser Session. Use browser_set_visibility to show the panel.")
        }
        if let rawURL = params["url"]?.stringValue, let url = URL(string: rawURL) {
            do {
                try await session.navigate(to: url)
            } catch {
                refreshCues()
                throw error
            }
        }
        return try json(session.pageInfo)
    }

    private func interactions(session: BrowserSession, params: [String: BrowserJSONValue]) async throws -> BrowserJSONValue {
        guard let values = params["actions"]?.arrayValue else {
            throw BrowserFailure(.invalidRequest, "page_interactions requires an actions array.")
        }
        let stopOnFailure = params["stop_on_failure"]?.boolValue ?? true
        var results: [BrowserJSONValue] = []
        for (index, value) in values.enumerated() {
            let action = try decodeAction(value, index: index)
            do {
                results.append(try json(try await session.interact(action)))
            } catch let failure as BrowserFailure {
                results.append(.object(["error": try json(failure)]))
                if stopOnFailure { throw failure }
            }
        }
        return .object([
            "results": .array(results),
            "page": try json(session.pageInfo),
        ])
    }

    private static let pageInteractionExpectedShape = "Actions are objects. Semantic actions use {type: click, ref: e_1_2, revision: 1}; type adds text, press adds key, and scroll adds delta_x and/or delta_y. Coordinate click, hover, move, and scroll use x and y."

    private func malformedAction(_ index: Int, _ message: String, field: String? = nil) -> BrowserFailure {
        var details = [
            "action_index": String(index),
            "expected": Self.pageInteractionExpectedShape,
            "semantic_click_example": "{type: click, ref: e_1_2, revision: 1}",
            "coordinate_click_example": "{type: click, x: 120, y: 80}",
        ]
        if let field { details["field"] = field }
        return BrowserFailure(.invalidRequest, message, details: details)
    }

    private func decodeAction(_ value: BrowserJSONValue, index: Int) throws -> BrowserAction {
        guard case .object(let object) = value else {
            throw malformedAction(index, "page_interactions action must be an object, not a string or other scalar.")
        }
        let allowedFields: Set<String> = ["type", "ref", "revision", "text", "key", "x", "y", "delta_x", "delta_y"]
        if let unknown = object.keys.first(where: { !allowedFields.contains($0) }) {
            throw malformedAction(index, "page_interactions action contains an unsupported field.", field: unknown)
        }
        guard let type = object["type"]?.stringValue else {
            throw malformedAction(index, "page_interactions action requires a string type.", field: "type")
        }

        func string(_ field: String) throws -> String? {
            guard let value = object[field] else { return nil }
            guard case .string(let string) = value else {
                throw malformedAction(index, "page_interactions action field has the wrong type.", field: field)
            }
            return string
        }
        func number(_ field: String) throws -> Double? {
            guard let value = object[field] else { return nil }
            guard let number = value.numberValue else {
                throw malformedAction(index, "page_interactions action field has the wrong type.", field: field)
            }
            return number
        }
        func revision() throws -> Int? {
            guard let value = object["revision"] else { return nil }
            guard let number = value.numberValue,
                  number.isFinite,
                  number.rounded() == number,
                  number >= 0,
                  number <= Double(Int.max) else {
                throw malformedAction(index, "page_interactions action revision must be a non-negative integer.", field: "revision")
            }
            return Int(number)
        }

        let action = BrowserAction(
            type: type,
            reference: try string("ref"),
            revision: try revision(),
            text: try string("text"),
            key: try string("key"),
            x: try number("x"),
            y: try number("y"),
            deltaX: try number("delta_x"),
            deltaY: try number("delta_y")
        )
        if let failure = action.validationFailure(actionIndex: index) {
            throw failure
        }
        return action
    }

    private func wait(session: BrowserSession, params: [String: BrowserJSONValue]) async throws -> BrowserJSONValue {
        let timeout = min(max(params["timeout_ms"]?.intValue ?? 10_000, 100), 60_000)
        let deadline = Date().addingTimeInterval(Double(timeout) / 1_000)
        while Date() < deadline {
            if let text = params["text"]?.stringValue,
               let snapshot = try? await session.snapshot(maxCharacters: 40_000),
               snapshot.visibleText.localizedCaseInsensitiveContains(text) {
                return .object(["matched": .string("text"), "page": try json(snapshot)])
            }
            if let pattern = params["url_pattern"]?.stringValue,
               session.pageInfo.url?.contains(pattern) == true {
                return .object(["matched": .string("url_pattern"), "page": try json(session.pageInfo)])
            }
            if let loadState = params["load_state"]?.stringValue,
               session.pageInfo.loadState.rawValue == loadState {
                return .object(["matched": .string("load_state"), "page": try json(session.pageInfo)])
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw BrowserFailure(.operationTimedOut, "browser_wait timed out.")
    }

    private func reclaimRequest(connectionID: String, connectionLabel: String) throws -> BrowserJSONValue {
        if registry.session(for: connectionID) != nil {
            throw BrowserFailure(.invalidRequest, "This Agent Connection already owns a Browser Session.")
        }
        guard let detached = sessions.values.first(where: { $0.isDetached }) else {
            throw BrowserFailure(.sessionNotFound, "There is no Detached Session available for reclaim.")
        }
        try detached.requestReclaim()
        reclaimRequests[detached.id] = (connectionID, connectionLabel)
        refreshCues()
        throw BrowserFailure(.approvalRequired, "The user must approve reclaim from the matching Detached Session Browser Cue.", details: ["session": detached.id.uuidString])
    }

    private func ownedSession(connectionID: String) throws -> BrowserSession {
        guard let sessionID = registry.session(for: connectionID), let session = sessions[sessionID] else {
            throw BrowserFailure(.sessionNotFound, "Open bagent Browser on this Agent Connection first.")
        }
        guard session.ownerConnectionID == connectionID else {
            throw BrowserFailure(.connectionNotOwner, "This Agent Connection does not own that Browser Session.")
        }
        return session
    }

    private func origin(for connectionID: String) -> String? {
        guard let sessionID = registry.session(for: connectionID) else { return nil }
        return sessions[sessionID]?.currentOrigin
    }

    private func refreshCues() {
        cues = sessions.values.map(\.cue).sorted { $0.id.uuidString < $1.id.uuidString }
    }

    private func json<T: Encodable>(_ value: T) throws -> BrowserJSONValue {
        let data = try JSONEncoder().encode(value)
        return try JSONDecoder().decode(BrowserJSONValue.self, from: data)
    }

}
