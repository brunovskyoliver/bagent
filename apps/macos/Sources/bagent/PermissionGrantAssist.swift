import AppKit
import ApplicationServices
import CoreGraphics
import Darwin
import Foundation
import UniformTypeIdentifiers

enum PermissionGrantKind: String, Codable, CaseIterable, Equatable, Sendable {
    case fullDiskAccess
    case screenRecording
    case accessibility
}

enum ProtectedPermissionResource: String, Codable, CaseIterable, Hashable, Sendable {
    case mail
    case notes
}

enum DaemonFullDiskAccessOutcome: String, Codable, Equatable, Sendable {
    case granted
    case denied
    case indeterminate
}

struct DaemonFullDiskAccessSnapshot: Codable, Equatable, Sendable {
    let mail: DaemonFullDiskAccessOutcome
    let notes: DaemonFullDiskAccessOutcome
}

protocol DaemonFullDiskAccessProbeAdapter: Sendable {
    func probe() async -> DaemonFullDiskAccessSnapshot
}

struct SystemDaemonFullDiskAccessProbe: DaemonFullDiskAccessProbeAdapter, Sendable {
    func probe() async -> DaemonFullDiskAccessSnapshot {
        await DaemonClient().fullDiskAccessProbe()
    }
}

enum PermissionProbeOutcome: String, Codable, Equatable, Sendable {
    case granted
    case denied
    case indeterminate
}

struct PermissionProbeSnapshot: Codable, Equatable, Sendable {
    let fullDiskAccess: [ProtectedPermissionResource: PermissionProbeOutcome]
    let screenRecording: PermissionProbeOutcome
    let accessibility: PermissionProbeOutcome
    let relaunchRequired: Set<PermissionGrantKind>

    init(
        fullDiskAccess: [ProtectedPermissionResource: PermissionProbeOutcome],
        screenRecording: PermissionProbeOutcome,
        accessibility: PermissionProbeOutcome,
        relaunchRequired: Set<PermissionGrantKind> = []
    ) {
        self.fullDiskAccess = fullDiskAccess
        self.screenRecording = screenRecording
        self.accessibility = accessibility
        self.relaunchRequired = relaunchRequired
    }

    func permissionState(for kind: PermissionGrantKind) -> PermissionProbeOutcome {
        switch kind {
        case .fullDiskAccess:
            let results = ProtectedPermissionResource.allCases.map { fullDiskAccess[$0] ?? .indeterminate }
            if results.allSatisfy({ $0 == .granted }) { return .granted }
            if results.contains(.denied) { return .denied }
            return .indeterminate
        case .screenRecording: return screenRecording
        case .accessibility: return accessibility
        }
    }

    func requiresUIRelaunch(for kind: PermissionGrantKind) -> Bool {
        relaunchRequired.contains(kind)
    }

    var needsDelayedPropagationRetry: Bool {
        PermissionGrantKind.allCases.contains {
            permissionState(for: $0) != .granted
        }
    }
}

protocol PermissionProbeAdapter: Sendable {
    func probe() async -> PermissionProbeSnapshot
}

/// Tracks process-scoped grants observed by this UI process. A transition from
/// missing to granted is authoritative, but the process that observed it must
/// not report itself active until a replacement process has rechecked it. The
/// daemon-owned FDA result is intentionally excluded: the daemon is the
/// process that opens Mail and Notes, so a UI restart cannot substitute for its
/// proof of access.
final class PermissionProbeLifecycle: @unchecked Sendable {
    private let lock = NSLock()
    private var last: [PermissionGrantKind: PermissionProbeOutcome] = [:]
    private var restartRequired = Set<PermissionGrantKind>()

    func relaunchRequirements(for snapshot: PermissionProbeSnapshot) -> Set<PermissionGrantKind> {
        lock.lock()
        defer { lock.unlock() }

        for kind in [PermissionGrantKind.screenRecording, .accessibility] {
            let result = snapshot.permissionState(for: kind)
            if result == .granted, last[kind].map({ $0 != .granted }) == true {
                restartRequired.insert(kind)
            }
            last[kind] = result
        }
        return restartRequired
    }
}

/// The production probe gets Full Disk Access from the daemon that reads Mail
/// and Notes. The UI process never opens those resources and cannot substitute
/// a successful UI probe for the daemon result.
struct SystemPermissionProbe: PermissionProbeAdapter, Sendable {
    let daemon: any DaemonFullDiskAccessProbeAdapter
    let screenRecording: @Sendable () -> Bool
    let accessibility: @Sendable () -> Bool
    let lifecycle: PermissionProbeLifecycle

    init(
        daemon: any DaemonFullDiskAccessProbeAdapter = SystemDaemonFullDiskAccessProbe(),
        screenRecording: @escaping @Sendable () -> Bool = CGPreflightScreenCaptureAccess,
        accessibility: @escaping @Sendable () -> Bool = AXIsProcessTrusted,
        lifecycle: PermissionProbeLifecycle = PermissionProbeLifecycle()
    ) {
        self.daemon = daemon
        self.screenRecording = screenRecording
        self.accessibility = accessibility
        self.lifecycle = lifecycle
    }

    func probe() async -> PermissionProbeSnapshot {
        let fullDiskAccess = await daemon.probe()
        let snapshot = PermissionProbeSnapshot(
            fullDiskAccess: [
                .mail: PermissionProbeOutcome(fullDiskAccess.mail),
                .notes: PermissionProbeOutcome(fullDiskAccess.notes),
            ],
            screenRecording: screenRecording() ? .granted : .denied,
            accessibility: accessibility() ? .granted : .denied,
            relaunchRequired: []
        )
        return PermissionProbeSnapshot(
            fullDiskAccess: snapshot.fullDiskAccess,
            screenRecording: snapshot.screenRecording,
            accessibility: snapshot.accessibility,
            relaunchRequired: lifecycle.relaunchRequirements(for: snapshot)
        )
    }
}

private extension PermissionProbeOutcome {
    init(_ result: DaemonFullDiskAccessOutcome) {
        switch result {
        case .granted: self = .granted
        case .denied: self = .denied
        case .indeterminate: self = .indeterminate
        }
    }
}

enum PermissionGrantAssistPhase: String, Codable, CaseIterable, Equatable, Sendable {
    case unknown
    case deniedOrMissing
    case openingExactPane
    case exactPaneFailureRootFallback
    case readyToDrag
    case draggingApplication
    case waitingForSystemSettings
    case authoritativeRecheck
    case grantedAndActive
    case grantedButUIRelaunchRequired
    case daemonPreservingRelaunchHandoff
    case relaunchCompletedAndPermissionRechecked
}

enum PermissionGrantAssistEvent: Equatable, Sendable {
    case exactPaneOpening
    case paneOpened
    case paneFailed
    case dragBegan
    case dragEnded
    case activation
    case probe(PermissionProbeOutcome)
    case relaunchRequested
    case replacementReady
    case relaunchProbe(PermissionProbeOutcome)
}

enum PermissionGrantAssistMachine {
    static func next(
        after phase: PermissionGrantAssistPhase,
        event: PermissionGrantAssistEvent
    ) -> PermissionGrantAssistPhase {
        switch (phase, event) {
        case (.deniedOrMissing, .exactPaneOpening): .openingExactPane
        case (.openingExactPane, .paneOpened): .readyToDrag
        case (.openingExactPane, .paneFailed): .exactPaneFailureRootFallback
        case (.exactPaneFailureRootFallback, .paneOpened): .readyToDrag
        case (.readyToDrag, .dragBegan): .draggingApplication
        case (.draggingApplication, .dragEnded): .waitingForSystemSettings
        case (.readyToDrag, .probe(.denied)), (.readyToDrag, .probe(.indeterminate)),
             (.draggingApplication, .probe(.denied)), (.draggingApplication, .probe(.indeterminate)),
             (.waitingForSystemSettings, .probe(.denied)), (.waitingForSystemSettings, .probe(.indeterminate)),
             (.grantedAndActive, .probe(.denied)), (.grantedAndActive, .probe(.indeterminate)):
            .deniedOrMissing
        case (.waitingForSystemSettings, .activation): .authoritativeRecheck
        case (.authoritativeRecheck, .probe(.granted)): .grantedAndActive
        case (.authoritativeRecheck, .probe(.denied)), (.authoritativeRecheck, .probe(.indeterminate)):
            .deniedOrMissing
        case (.unknown, .probe(.granted)): .grantedAndActive
        case (.unknown, .probe(.denied)), (.unknown, .probe(.indeterminate)): .deniedOrMissing
        case (.grantedAndActive, .relaunchRequested): .daemonPreservingRelaunchHandoff
        case (.daemonPreservingRelaunchHandoff, .replacementReady): .authoritativeRecheck
        case (.authoritativeRecheck, .relaunchProbe(.granted)): .relaunchCompletedAndPermissionRechecked
        case (.authoritativeRecheck, .relaunchProbe(.denied)), (.authoritativeRecheck, .relaunchProbe(.indeterminate)):
            .deniedOrMissing
        default: phase
        }
    }

    static func phase(
        after result: PermissionProbeOutcome,
        uiRequiresRelaunch: Bool
    ) -> PermissionGrantAssistPhase {
        guard result == .granted else { return .deniedOrMissing }
        return uiRequiresRelaunch ? .grantedButUIRelaunchRequired : .grantedAndActive
    }
}

@MainActor
final class PermissionRecheckCoordinator {
    private let probe: any PermissionProbeAdapter
    private let debounce: Duration
    private let onSnapshot: @MainActor (PermissionProbeSnapshot) -> Void
    private var generation = 0
    private var task: Task<Void, Never>?
    private(set) var phase: PermissionGrantAssistPhase = .unknown
    private(set) var latestSnapshot: PermissionProbeSnapshot?

    init(
        probe: any PermissionProbeAdapter,
        debounce: Duration = .milliseconds(250),
        onSnapshot: @escaping @MainActor (PermissionProbeSnapshot) -> Void = { _ in }
    ) {
        self.probe = probe
        self.debounce = debounce
        self.onSnapshot = onSnapshot
    }

    func probeInitially() {
        scheduleProbe()
    }

    func didBecomeActive() {
        phase = .authoritativeRecheck
        scheduleProbe()
    }

    func probeAuthoritatively() async -> PermissionProbeSnapshot? {
        generation += 1
        let expectedGeneration = generation
        task?.cancel()
        let snapshot = await probe.probe()
        guard generation == expectedGeneration else { return nil }
        latestSnapshot = snapshot
        onSnapshot(snapshot)
        phase = PermissionGrantAssistMachine.phase(
            after: snapshot.permissionState(for: .fullDiskAccess),
            uiRequiresRelaunch: snapshot.requiresUIRelaunch(for: .fullDiskAccess)
        )
        return snapshot
    }

    func stop() {
        generation += 1
        task?.cancel()
        task = nil
        phase = .unknown
        latestSnapshot = nil
    }

    private func scheduleProbe() {
        generation += 1
        let expectedGeneration = generation
        task?.cancel()
        task = Task { [weak self] in
            do {
                try await Task.sleep(for: self?.debounce ?? .zero)
            } catch {
                return
            }
            guard let self, !Task.isCancelled else { return }
            var snapshot = await self.probe.probe()
            for delay in [Duration.milliseconds(100), .milliseconds(250)]
                where snapshot.needsDelayedPropagationRetry
            {
                do {
                    try await Task.sleep(for: delay)
                } catch {
                    return
                }
                guard !Task.isCancelled, self.generation == expectedGeneration else { return }
                snapshot = await self.probe.probe()
            }
            guard !Task.isCancelled, self.generation == expectedGeneration else { return }
            self.latestSnapshot = snapshot
            self.onSnapshot(snapshot)
            self.phase = PermissionGrantAssistMachine.phase(
                after: snapshot.permissionState(for: .fullDiskAccess),
                uiRequiresRelaunch: snapshot.requiresUIRelaunch(for: .fullDiskAccess)
            )
        }
    }
}

enum PermissionSystemSettingsRoute: String, CaseIterable, Equatable, Sendable {
    case privacyAndSecurityRoot
    case fullDiskAccess
    case screenRecording
    case accessibility

    static func destination(for route: Self) -> URL {
        switch route {
        case .privacyAndSecurityRoot:
            URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension")!
        case .fullDiskAccess:
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")!
        case .screenRecording:
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!
        case .accessibility:
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!
        }
    }

    static func expectedTitles(for route: Self, osMajor: Int) -> [String] {
        switch route {
        case .privacyAndSecurityRoot: ["Privacy & Security"]
        case .fullDiskAccess: ["Full Disk Access"]
        case .screenRecording:
            osMajor >= 26 ? ["Screen & System Audio Recording"] : ["Screen Recording", "Screen & System Audio Recording"]
        case .accessibility: ["Accessibility"]
        }
    }
}

protocol PermissionSettingsOpener {
    func open(_ url: URL) -> Bool
    func confirmsExpectedPane(_ route: PermissionSystemSettingsRoute) -> Bool
}

enum PermissionRouteResult: Equatable {
    case openedExact(PermissionSystemSettingsRoute)
    case fallbackToRoot(PermissionSystemSettingsRoute)
    case failed(PermissionSystemSettingsRoute)

    var establishesPermission: Bool { false }
}

enum PermissionSettingsRouter {
    static func open(_ kind: PermissionGrantKind, opener: PermissionSettingsOpener) -> PermissionRouteResult {
        let route = systemRoute(for: kind)
        guard opener.open(PermissionSystemSettingsRoute.destination(for: route)) else {
            return .failed(route)
        }
        if opener.confirmsExpectedPane(route) {
            return .openedExact(route)
        }
        let root = PermissionSystemSettingsRoute.privacyAndSecurityRoot
        guard opener.open(PermissionSystemSettingsRoute.destination(for: root)) else {
            return .failed(route)
        }
        return .fallbackToRoot(route)
    }

    static func confirmation(
        requested: PermissionSystemSettingsRoute,
        observedTitle: String,
        osMajor: Int = ProcessInfo.processInfo.operatingSystemVersion.majorVersion
    ) -> PermissionRouteConfirmation {
        PermissionSystemSettingsRoute.expectedTitles(for: requested, osMajor: osMajor).contains(observedTitle)
            ? .confirmed : .unconfirmed
    }

    private static func systemRoute(for kind: PermissionGrantKind) -> PermissionSystemSettingsRoute {
        switch kind {
        case .fullDiskAccess: .fullDiskAccess
        case .screenRecording: .screenRecording
        case .accessibility: .accessibility
        }
    }
}

enum PermissionRouteConfirmation: Equatable {
    case confirmed
    case unconfirmed
}

struct ApplicationSignature: Equatable, Sendable {
    let identifier: String?
    let teamIdentifier: String?
    let isAdHoc: Bool
    let isValid: Bool

    static func valid(identifier: String, teamID: String) -> Self {
        Self(identifier: identifier, teamIdentifier: teamID, isAdHoc: false, isValid: true)
    }

    static let adHoc = Self(identifier: "sk.bagent.app", teamIdentifier: nil, isAdHoc: true, isValid: true)
    static let invalid = Self(identifier: nil, teamIdentifier: nil, isAdHoc: false, isValid: false)
}

enum ApplicationDragValidationResult: Equatable {
    case valid
    case missingBundle
    case notApplicationBundle
    case wrongIdentifier
    case wrongTeam
    case adHocSignature
    case invalidSignature
    case alias
    case filePromise
}

enum ApplicationDragValidator {
    static func validate(
        appURL: URL,
        expectedIdentifier: String,
        expectedTeamID: String,
        signature: ApplicationSignature
    ) -> ApplicationDragValidationResult {
        let fileManager = FileManager.default
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: appURL.path, isDirectory: &isDirectory) else { return .missingBundle }
        guard isDirectory.boolValue, appURL.pathExtension.caseInsensitiveCompare("app") == .orderedSame else {
            return .notApplicationBundle
        }
        guard let identifier = Bundle(url: appURL)?.bundleIdentifier, identifier == expectedIdentifier else {
            return .wrongIdentifier
        }
        guard signature.isValid else { return .invalidSignature }
        guard !signature.isAdHoc else { return .adHocSignature }
        guard signature.identifier == expectedIdentifier else { return .wrongIdentifier }
        guard signature.teamIdentifier == expectedTeamID else { return .wrongTeam }
        return .valid
    }

    static func rejection(for url: URL, isAlias: Bool = false, isFilePromise: Bool = false) -> ApplicationDragValidationResult {
        if isAlias { return .alias }
        if isFilePromise { return .filePromise }
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
              isDirectory.boolValue,
              url.pathExtension.caseInsensitiveCompare("app") == .orderedSame else {
            return .notApplicationBundle
        }
        return .valid
    }
}

enum ApplicationDragRepresentation {
    static let contentTypes = [UTType.fileURL.identifier]
    static let accessibilityLabel = "Draggable bagent application"
    static let accessibilityHint = "Drag the signed running application into the selected System Settings pane."
    static let nativeAddFlowHint = "If dragging is unavailable, use System Settings' native Add flow."

    static func fileURL(for bundleURL: URL) -> URL {
        URL(fileURLWithPath: bundleURL.path, isDirectory: false)
    }

    static func writer(for bundleURL: URL) -> NSURL {
        NSURL(fileURLWithPath: fileURL(for: bundleURL).path)
    }

    static func validatedDragItem(
        for bundleURL: URL,
        expectedIdentifier: String = "sk.bagent.app",
        expectedTeamID: String = "QUB47S3XTF"
    ) -> NSItemProvider? {
        let signature = ApplicationSignatureInspector.inspect(bundleURL)
        guard ApplicationDragValidator.validate(
            appURL: bundleURL,
            expectedIdentifier: expectedIdentifier,
            expectedTeamID: expectedTeamID,
            signature: signature
        ) == .valid else { return nil }
        return NSItemProvider(object: writer(for: bundleURL))
    }

    static func hasValidSignedBundle(
        at bundleURL: URL,
        expectedIdentifier: String = "sk.bagent.app",
        expectedTeamID: String = "QUB47S3XTF"
    ) -> Bool {
        let signature = ApplicationSignatureInspector.inspect(bundleURL)
        return ApplicationDragValidator.validate(
            appURL: bundleURL,
            expectedIdentifier: expectedIdentifier,
            expectedTeamID: expectedTeamID,
            signature: signature
        ) == .valid
    }
}

enum ApplicationSignatureInspector {
    static func inspect(_ appURL: URL) -> ApplicationSignature {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
        process.arguments = ["--display", "--verbose=4", "--strict", appURL.path]
        let pipe = Pipe()
        process.standardError = pipe
        process.standardOutput = FileHandle.nullDevice
        do {
            try process.run()
            process.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            let output = String(decoding: data, as: UTF8.self)
            guard process.terminationStatus == 0 else { return .invalid }
            let identifier = value(after: "Identifier=", in: output)
            let team = value(after: "TeamIdentifier=", in: output)
            return ApplicationSignature(
                identifier: identifier,
                teamIdentifier: team,
                isAdHoc: output.contains("Signature=adhoc"),
                isValid: true
            )
        } catch {
            return .invalid
        }
    }

    private static func value(after prefix: String, in output: String) -> String? {
        output.split(separator: "\n").first { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
    }
}

enum UIOnlyRelaunchEligibility {
    static let automationSurvives = true

    static func isAllowed(activeConversationTurn: Bool, pendingApproval: Bool) -> Bool {
        !activeConversationTurn && !pendingApproval
    }
}

enum AppLaunchMode: Equatable, Sendable {
    case ordinary
    case uiOnlyRelaunch(token: String)
    case invalidUIOnlyRelaunch
    case stage7CAcceptanceOld

    static func parse(arguments: [String]) -> Self {
        if arguments.count == 2, arguments[1] == "--stage7c-acceptance-old" {
            return .stage7CAcceptanceOld
        }
        guard arguments.count > 1, arguments[1] == "--ui-relaunch-token" else {
            return .ordinary
        }
        guard arguments.count == 3, !arguments[2].isEmpty else {
            return .invalidUIOnlyRelaunch
        }
        return .uiOnlyRelaunch(token: arguments[2])
    }

    var startsDaemon: Bool {
        if case .ordinary = self { return true }
        return false
    }

    var startsMonitoring: Bool {
        if case .uiOnlyRelaunch = self { return false }
        if case .invalidUIOnlyRelaunch = self { return false }
        return true
    }

    var isStage7CAcceptanceOld: Bool {
        if case .stage7CAcceptanceOld = self { return true }
        return false
    }
}

struct UIRelaunchHandoff: Codable, Equatable, Sendable {
    static let schemaVersion = 2
    static let lifetime: TimeInterval = 60
    static let maximumDraftBytes = 16 * 1024

    let schemaVersion: Int
    let createdAt: Date
    let expiresAt: Date
    let nonce: String
    let sourceUIIdentity: String
    let replacementUIIdentity: String
    let sourceConsumerFence: String
    let replacementConsumerFence: String
    let currentChatIdentity: String
    let refetchCursor: UInt64?
    let draft: String
    let caretOffset: Int
    let selectionLength: Int
    let pendingAttachmentReferences: [String]
    let selectedArea: CompassRailArea
    let selectedChild: CompassRailChild?
    let permissionPhase: PermissionGrantAssistPhase
    let semanticFocus: String

    func validate(now: Date) throws {
        guard schemaVersion == Self.schemaVersion else { throw UIRelaunchHandoffError.unknownVersion }
        guard now < expiresAt else { throw UIRelaunchHandoffError.expired }
        guard expiresAt == createdAt.addingTimeInterval(Self.lifetime) else { throw UIRelaunchHandoffError.malformed }
        guard !nonce.isEmpty, !sourceUIIdentity.isEmpty, !replacementUIIdentity.isEmpty,
              !sourceConsumerFence.isEmpty, !replacementConsumerFence.isEmpty,
              !currentChatIdentity.isEmpty else { throw UIRelaunchHandoffError.invalidIdentity }
        guard draft.utf8.count <= Self.maximumDraftBytes else { throw UIRelaunchHandoffError.oversizedDraft }
        guard caretOffset >= 0, selectionLength >= 0,
              caretOffset + selectionLength <= draft.utf16.count else {
            throw UIRelaunchHandoffError.invalidSelection
        }
        guard pendingAttachmentReferences.count <= 16 else { throw UIRelaunchHandoffError.invalidAttachments }
    }

    init(
        createdAt: Date,
        sourceUIIdentity: String,
        replacementUIIdentity: String,
        sourceConsumerFence: String = UUID().uuidString,
        replacementConsumerFence: String = UUID().uuidString,
        currentChatIdentity: String,
        refetchCursor: UInt64?,
        draft: String,
        caretOffset: Int,
        selectionLength: Int,
        pendingAttachmentReferences: [String],
        selectedArea: CompassRailArea,
        selectedChild: CompassRailChild?,
        permissionPhase: PermissionGrantAssistPhase,
        semanticFocus: String,
        nonce: String = UUID().uuidString
    ) throws {
        guard draft.utf8.count <= Self.maximumDraftBytes else { throw UIRelaunchHandoffError.oversizedDraft }
        guard caretOffset >= 0, selectionLength >= 0 else { throw UIRelaunchHandoffError.invalidSelection }
        guard caretOffset + selectionLength <= draft.utf16.count else { throw UIRelaunchHandoffError.invalidSelection }
        guard pendingAttachmentReferences.count <= 16 else { throw UIRelaunchHandoffError.invalidAttachments }
        guard !sourceUIIdentity.isEmpty, !replacementUIIdentity.isEmpty, !currentChatIdentity.isEmpty else {
            throw UIRelaunchHandoffError.invalidIdentity
        }
        guard !sourceConsumerFence.isEmpty, !replacementConsumerFence.isEmpty,
              sourceConsumerFence != replacementConsumerFence else {
            throw UIRelaunchHandoffError.invalidIdentity
        }
        self.schemaVersion = Self.schemaVersion
        self.createdAt = createdAt
        self.expiresAt = createdAt.addingTimeInterval(Self.lifetime)
        self.nonce = nonce
        self.sourceUIIdentity = sourceUIIdentity
        self.replacementUIIdentity = replacementUIIdentity
        self.sourceConsumerFence = sourceConsumerFence
        self.replacementConsumerFence = replacementConsumerFence
        self.currentChatIdentity = currentChatIdentity
        self.refetchCursor = refetchCursor
        self.draft = draft
        self.caretOffset = caretOffset
        self.selectionLength = selectionLength
        self.pendingAttachmentReferences = pendingAttachmentReferences
        self.selectedArea = selectedArea
        self.selectedChild = selectedChild
        self.permissionPhase = permissionPhase
        self.semanticFocus = semanticFocus
    }
}

enum UIRelaunchHandoffError: Error, Equatable {
    case unknownVersion
    case expired
    case replayed
    case identityMismatch
    case oversizedDraft
    case invalidSelection
    case invalidAttachments
    case invalidIdentity
    case malformed
}

enum UIRelaunchHandoffCodec {
    static func encode(_ handoff: UIRelaunchHandoff) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(handoff)
    }

    static func decode(_ data: Data, now: Date) throws -> UIRelaunchHandoff {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard let handoff = try? decoder.decode(UIRelaunchHandoff.self, from: data) else {
            throw UIRelaunchHandoffError.malformed
        }
        try handoff.validate(now: now)
        return handoff
    }
}

final class InMemoryProtectedHandoffStore {
    private var payloads: [String: Data] = [:]
    private let lock = NSLock()

    func write(_ handoff: UIRelaunchHandoff) throws -> String {
        let token = UUID().uuidString
        let data = try UIRelaunchHandoffCodec.encode(handoff)
        lock.lock(); defer { lock.unlock() }
        payloads[token] = data
        return token
    }

    func consume(token: String, source: String, replacement: String, now: Date) throws -> UIRelaunchHandoff {
        lock.lock(); defer { lock.unlock() }
        guard let data = payloads[token] else { throw UIRelaunchHandoffError.replayed }
        let handoff: UIRelaunchHandoff
        do {
            handoff = try UIRelaunchHandoffCodec.decode(data, now: now)
        } catch UIRelaunchHandoffError.expired {
            payloads.removeValue(forKey: token)
            throw UIRelaunchHandoffError.expired
        }
        guard handoff.sourceUIIdentity == source, handoff.replacementUIIdentity == replacement else {
            throw UIRelaunchHandoffError.identityMismatch
        }
        payloads.removeValue(forKey: token)
        return handoff
    }
}

@MainActor
final class KeychainProtectedHandoffStore {
    private let keyPrefix = "ui-relaunch-handoff."

    func write(_ handoff: UIRelaunchHandoff) throws -> String {
        let token = UUID().uuidString
        let data = try UIRelaunchHandoffCodec.encode(handoff)
        guard KeychainStore.save(key: keyPrefix + token, value: data.base64EncodedString()) else {
            throw UIRelaunchHandoffError.malformed
        }
        return token
    }

    func consume(token: String, replacement: String, now: Date) throws -> UIRelaunchHandoff {
        let lock = try HandoffKeychainLock()
        defer { _ = lock }
        guard let encoded = KeychainStore.load(key: keyPrefix + token),
              let data = Data(base64Encoded: encoded) else {
            throw UIRelaunchHandoffError.replayed
        }
        let handoff: UIRelaunchHandoff
        do {
            handoff = try UIRelaunchHandoffCodec.decode(data, now: now)
        } catch UIRelaunchHandoffError.expired {
            KeychainStore.delete(key: keyPrefix + token)
            throw UIRelaunchHandoffError.expired
        }
        guard handoff.replacementUIIdentity == replacement else {
            throw UIRelaunchHandoffError.identityMismatch
        }
        KeychainStore.delete(key: keyPrefix + token)
        return handoff
    }
}

/// Serializes the Keychain read-and-delete pair across UI processes. The lock
/// file contains no handoff data and is only a cross-process mutex.
private final class HandoffKeychainLock {
    private let descriptor: Int32

    init() throws {
        let directory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("sk.bagent.app", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let path = directory.appendingPathComponent("ui-relaunch-keychain.lock").path
        descriptor = Darwin.open(path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        var writeLock = Darwin.flock(
            l_start: 0,
            l_len: 0,
            l_pid: 0,
            l_type: Int16(F_WRLCK),
            l_whence: Int16(SEEK_SET)
        )
        guard descriptor >= 0, Darwin.fcntl(descriptor, F_SETLKW, &writeLock) == 0 else {
            if descriptor >= 0 { _ = Darwin.close(descriptor) }
            throw UIRelaunchHandoffError.malformed
        }
    }

    deinit {
        var unlock = Darwin.flock(
            l_start: 0,
            l_len: 0,
            l_pid: 0,
            l_type: Int16(F_UNLCK),
            l_whence: Int16(SEEK_SET)
        )
        _ = Darwin.fcntl(descriptor, F_SETLK, &unlock)
        _ = Darwin.close(descriptor)
    }
}

@MainActor
final class UIOnlyRelaunchCoordinator {
    private let store: KeychainProtectedHandoffStore

    init(store: KeychainProtectedHandoffStore = KeychainProtectedHandoffStore()) {
        self.store = store
    }

    /// Only the opaque token crosses the process boundary. The allowlisted
    /// handoff remains in the user-unlocked Keychain until one consumption.
    func launchReplacement(
        handoff: UIRelaunchHandoff,
        applicationURL: URL,
        completion: @escaping @MainActor @Sendable (Result<String, UIOnlyRelaunchLaunchError>) -> Void
    ) {
        do {
            let token = try store.write(handoff)
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = false
            configuration.createsNewApplicationInstance = true
            configuration.addsToRecentItems = false
            configuration.arguments = ["--ui-relaunch-token", token]
            var environment = ProcessInfo.processInfo.environment
            for key in ["BAGENT_STAGE7C_ACCEPTANCE_FIXTURE", "BAGENT_DATA_DIR", "BAGENT_STAGE7C_EVIDENCE_DIR", "BAGENT_STAGE7C_FDA_FIXTURE"] {
                if let value = ProcessInfo.processInfo.environment[key] {
                    environment[key] = value
                }
            }
            configuration.environment = environment
            if ProcessInfo.processInfo.environment["BAGENT_STAGE7C_ACCEPTANCE_FIXTURE"] == "1" {
                let process = Process()
                process.executableURL = applicationURL.appendingPathComponent("Contents/MacOS/bagent")
                process.arguments = ["--ui-relaunch-token", token]
                process.environment = environment
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.nullDevice
                try process.run()
                completion(.success(token))
                return
            }
            NSWorkspace.shared.openApplication(at: applicationURL, configuration: configuration) { _, error in
                DispatchQueue.main.async {
                    if let error {
                        completion(.failure(.failed(error.localizedDescription)))
                    } else {
                        completion(.success(token))
                    }
                }
            }
        } catch {
            completion(.failure(.failed(error.localizedDescription)))
        }
    }
}

enum UIOnlyRelaunchLaunchError: Error, Equatable, Sendable {
    case failed(String)
}

enum UIOnlyRelaunchAction: Hashable {
    case buildHandoff
    case launchReplacement
    case consumeHandoff
    case refetch
    case fenceUI
    case activateReplacement
    case probe
    case launchDaemon
    case restartBaseRT
    case mutateAutomationWork
}

enum UIOnlyRelaunchOwnership {
    static let allowedActions: [UIOnlyRelaunchAction] = [
        .buildHandoff, .launchReplacement, .consumeHandoff, .refetch,
        .fenceUI, .activateReplacement, .probe,
    ]
    static let forbiddenActions: Set<UIOnlyRelaunchAction> = [
        .launchDaemon, .restartBaseRT, .mutateAutomationWork,
    ]
}

enum UIRelaunchTransferPhase: Equatable, Sendable {
    case oldActive
    case replacementHidden
    case handoffConsumed
    case stateRefetched
    case authorityReserved
    case replacementReady
    case oldUIFenced
    case successorActive
    case acknowledged
    case rolledBack
    case lateReplacementExited
}

struct UIRelaunchTransferState: Equatable, Sendable {
    let phase: UIRelaunchTransferPhase
    let oldVisible: Bool
    let oldInteractive: Bool
    let oldConsumes: Bool
    let replacementVisible: Bool
    let replacementInteractive: Bool
    let replacementConsumes: Bool

    init(phase: UIRelaunchTransferPhase) {
        self.phase = phase
        switch phase {
        case .oldActive, .replacementHidden, .handoffConsumed, .stateRefetched,
             .authorityReserved, .replacementReady:
            oldVisible = true
            oldInteractive = true
            oldConsumes = true
            replacementVisible = false
            replacementInteractive = false
            replacementConsumes = false
        case .oldUIFenced:
            oldVisible = false
            oldInteractive = false
            oldConsumes = false
            replacementVisible = false
            replacementInteractive = false
            replacementConsumes = false
        case .successorActive, .acknowledged:
            oldVisible = false
            oldInteractive = false
            oldConsumes = false
            replacementVisible = true
            replacementInteractive = true
            replacementConsumes = true
        case .rolledBack, .lateReplacementExited:
            oldVisible = true
            oldInteractive = true
            oldConsumes = true
            replacementVisible = false
            replacementInteractive = false
            replacementConsumes = false
        }
    }
}

enum UIRelaunchTransferEvent: Equatable, Sendable {
    case replacementLaunched
    case handoffConsumed
    case authoritativeStateRefetched
    case successorAuthorityReserved
    case replacementReady
    case oldUIFenced
    case successorActivated
    case activePresentationAcknowledged
    case takeoverTimedOut
    case lateReplacementDetected
    case replacementCrashed
    case failedReadiness
    case failedActivationAcknowledgement
    case staleConsumerRejected
    case duplicateReplacementRejected
    case tokenReplayRejected
    case daemonUnavailable
    case daemonAvailable
}

enum UIRelaunchTransferError: Error, Equatable {
    case invalidTransition
    case rejected
}

struct UIRelaunchTransferMachine: Sendable {
    static let timeout: TimeInterval = 10

    private(set) var state = UIRelaunchTransferState(phase: .oldActive)

    mutating func apply(_ event: UIRelaunchTransferEvent) throws {
        let next: UIRelaunchTransferPhase?
        switch (state.phase, event) {
        case (.oldActive, .replacementLaunched): next = .replacementHidden
        case (.replacementHidden, .handoffConsumed): next = .handoffConsumed
        case (.handoffConsumed, .authoritativeStateRefetched): next = .stateRefetched
        case (.stateRefetched, .successorAuthorityReserved): next = .authorityReserved
        case (.authorityReserved, .replacementReady): next = .replacementReady
        case (.replacementReady, .oldUIFenced): next = .oldUIFenced
        case (.oldUIFenced, .successorActivated): next = .successorActive
        case (.successorActive, .activePresentationAcknowledged): next = .acknowledged
        case (.authorityReserved, .replacementCrashed),
             (.authorityReserved, .failedReadiness),
             (.replacementReady, .replacementCrashed),
             (.replacementReady, .failedReadiness),
             (.oldUIFenced, .replacementCrashed),
             (.successorActive, .failedActivationAcknowledgement),
             (.replacementHidden, .takeoverTimedOut),
             (.handoffConsumed, .takeoverTimedOut),
             (.stateRefetched, .takeoverTimedOut),
             (.authorityReserved, .takeoverTimedOut),
             (.replacementReady, .takeoverTimedOut),
             (.oldUIFenced, .takeoverTimedOut),
             (.successorActive, .takeoverTimedOut):
            next = .rolledBack
        case (.rolledBack, .lateReplacementDetected): next = .lateReplacementExited
        case (_, .daemonUnavailable), (_, .daemonAvailable): next = state.phase
        case (_, .staleConsumerRejected), (_, .duplicateReplacementRejected), (_, .tokenReplayRejected):
            throw UIRelaunchTransferError.rejected
        case (.oldActive, .replacementCrashed), (.oldActive, .failedReadiness),
             (.oldActive, .failedActivationAcknowledgement):
            throw UIRelaunchTransferError.rejected
        default:
            throw UIRelaunchTransferError.invalidTransition
        }
        state = UIRelaunchTransferState(phase: next!)
    }
}
