import AppKit
import ApplicationServices
import CoreGraphics
import SwiftUI

@MainActor
final class PermissionsManager: ObservableObject {
    @Published private(set) var hasFullDiskAccess = false
    @Published private(set) var hasScreenRecording = false
    @Published private(set) var hasAccessibility = false
    @Published private(set) var fullDiskAccessResources: [ProtectedPermissionResource: PermissionProbeOutcome] = [:]
    @Published private(set) var assistPhase: [PermissionGrantKind: PermissionGrantAssistPhase] = [:]
    @Published private(set) var lastRouteResult: PermissionRouteResult?

    private let probe: any PermissionProbeAdapter
    private let probesOnAssistStart: Bool
    private lazy var recheckCoordinator: PermissionRecheckCoordinator = PermissionRecheckCoordinator(probe: probe) { [weak self] snapshot in
        self?.apply(snapshot)
    }
    private var activationObserver: PermissionActivationObserver?

    init(
        probe: any PermissionProbeAdapter = SystemPermissionProbe(),
        probesOnAssistStart: Bool = true
    ) {
        self.probe = probe
        self.probesOnAssistStart = probesOnAssistStart
        let observer = PermissionActivationObserver { [weak self] in
            self?.markAuthoritativeRecheck()
            self?.recheckCoordinator.didBecomeActive()
        } onEnd: { [weak self] in
            self?.recheckCoordinator.stop()
        }
        NotificationCenter.default.addObserver(
            observer,
            selector: #selector(PermissionActivationObserver.didBecomeActive(_:)),
            name: NSApplication.didBecomeActiveNotification,
            object: nil
        )
        activationObserver = observer
    }

    /// Starts the nonprompting authoritative probe. Opening a pane or a
    /// request callback never changes these values.
    func refresh() {
        recheckCoordinator.probeInitially()
    }

    func refreshAuthoritatively() async -> PermissionProbeSnapshot? {
        await recheckCoordinator.probeAuthoritatively()
    }

    func beginAssist(for kind: PermissionGrantKind) {
        guard assistPhase[kind] == nil else { return }
        assistPhase[kind] = .unknown
        if probesOnAssistStart {
            recheckCoordinator.probeInitially()
        }
    }

    func openPrivacySettings() { open(.fullDiskAccess) }
    func openScreenRecordingSettings() { open(.screenRecording) }
    func openAccessibilitySettings() { open(.accessibility) }

    func requestScreenRecording() {
        _ = CGRequestScreenCaptureAccess()
        open(.screenRecording)
    }

    func requestAccessibility() {
        let key = "AXTrustedCheckOptionPrompt" as CFString
        _ = AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
        open(.accessibility)
    }

    func open(_ kind: PermissionGrantKind) {
        assistPhase[kind] = .openingExactPane
        let result = PermissionSettingsRouter.open(kind, opener: LiveSettingsOpener())
        lastRouteResult = result
        switch result {
        case .openedExact:
            assistPhase[kind] = .readyToDrag
        case .fallbackToRoot, .failed:
            assistPhase[kind] = .exactPaneFailureRootFallback
        }
    }

    func phase(for kind: PermissionGrantKind) -> PermissionGrantAssistPhase {
        assistPhase[kind] ?? .unknown
    }

    func restore(phase: PermissionGrantAssistPhase, for kind: PermissionGrantKind) {
        assistPhase[kind] = phase
    }

    /// Records the replacement process's own authoritative result. A launch,
    /// route, or daemon response alone never clears the relaunch state.
    func applyReplacementProbe(_ snapshot: PermissionProbeSnapshot, for kind: PermissionGrantKind) {
        let result = snapshot.permissionState(for: kind)
        guard result == .granted else {
            assistPhase[kind] = .deniedOrMissing
            return
        }
        assistPhase[kind] = snapshot.requiresUIRelaunch(for: kind)
            ? .grantedButUIRelaunchRequired
            : .relaunchCompletedAndPermissionRechecked
    }

    func dragBegan(for kind: PermissionGrantKind) {
        guard phase(for: kind) == .readyToDrag else { return }
        assistPhase[kind] = .draggingApplication
    }

    func dragEnded(for kind: PermissionGrantKind) {
        guard phase(for: kind) == .draggingApplication else { return }
        assistPhase[kind] = .waitingForSystemSettings
    }

    private func markAuthoritativeRecheck() {
        for kind in PermissionGrantKind.allCases {
            assistPhase[kind] = .authoritativeRecheck
        }
    }

    private func apply(_ snapshot: PermissionProbeSnapshot) {
        fullDiskAccessResources = snapshot.fullDiskAccess
        hasFullDiskAccess = snapshot.permissionState(for: .fullDiskAccess) == .granted
        hasScreenRecording = snapshot.screenRecording == .granted
        hasAccessibility = snapshot.accessibility == .granted
        for kind in PermissionGrantKind.allCases {
            let result = snapshot.permissionState(for: kind)
            if assistPhase[kind] == nil || assistPhase[kind] == .unknown || assistPhase[kind] == .authoritativeRecheck {
                assistPhase[kind] = PermissionGrantAssistMachine.phase(
                    after: result,
                    uiRequiresRelaunch: snapshot.requiresUIRelaunch(for: kind)
                )
            } else if result != .granted {
                assistPhase[kind] = .deniedOrMissing
            }
        }
    }
}

private final class PermissionActivationObserver: NSObject {
    private let onActivation: @MainActor () -> Void
    private let onEnd: @MainActor () -> Void

    init(onActivation: @escaping @MainActor () -> Void, onEnd: @escaping @MainActor () -> Void) {
        self.onActivation = onActivation
        self.onEnd = onEnd
        super.init()
    }

    @objc func didBecomeActive(_ notification: Notification) {
        Task { @MainActor [onActivation] in
            onActivation()
        }
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        Task { @MainActor [onEnd] in
            onEnd()
        }
    }
}

private struct LiveSettingsOpener: PermissionSettingsOpener {
    func open(_ url: URL) -> Bool { NSWorkspace.shared.open(url) }

    /// LaunchServices success does not prove the pane. The live acceptance
    /// fixture supplies confirmation; production safely falls back to root.
    func confirmsExpectedPane(_: PermissionSystemSettingsRoute) -> Bool { false }
}
