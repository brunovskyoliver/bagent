import AppKit
import Foundation

// MARK: - Model

enum CmuxEventKind {
    case attention
    case finished
}

/// A cue the user has acknowledged, snapshotted so the notch can fly its icon
/// off toward the notch (or off the top of an external screen) after the pending
/// entry itself is gone.
struct CmuxDeparture: Identifiable, Equatable {
    let id = UUID()
    let kind: CmuxEventKind
}

struct CmuxNotification: Identifiable, Equatable {
    let id: UUID
    let kind: CmuxEventKind
    let hookName: String
    let workspaceId: String
    let surfaceId: String?
    let windowId: String?
    let sessionId: String?
    var workspaceName: String?
    let occurredAt: Date

    /// One agent instance = one pending slot. Codex/Claude sessions carry a
    /// session_id, so two agents in the same workspace stay distinct.
    var dedupeKey: String { sessionId ?? workspaceId }

    var typeLabel: String {
        switch hookName {
        case "AskUserQuestion":    return "Question"
        case "ExitPlanMode":       return "Plan ready"
        case "PermissionRequest":  return "Approval needed"
        case "Notification":       return "Needs attention"
        case "Stop":               return "Finished"
        default:                   return kind == .attention ? "Needs attention" : "Finished"
        }
    }

    var title: String {
        if let name = workspaceName, !name.isEmpty {
            return "\(typeLabel) · \(name)"
        }
        return typeLabel
    }
}

// MARK: - Event monitor

/// Observes the cmux event bus by spawning `cmux events --reconnect` and parsing
/// its NDJSON stream. Delivers attention/finished agent events on the main actor
/// and can route focus back to the originating cmux workspace/tab.
@MainActor
final class CmuxEventMonitor {
    static let bundleIdentifier = "com.cmuxterm.app"
    private nonisolated static let appPath = "/Applications/cmux.app"

    /// Hook events that surface in the notch. Every hook is emitted twice
    /// (phase "received" then "completed") — only "received" is consumed.
    private static let attentionHooks: Set<String> = [
        "Notification", "AskUserQuestion", "ExitPlanMode", "PermissionRequest",
    ]
    private static let finishedHooks: Set<String> = ["Stop"]

    /// Tool calls that mean "agent is waiting for the user" when they surface
    /// as PreToolUse: codex questions use request_user_input; Claude's
    /// question/plan tools keep their names.
    private static let attentionToolNames: [String: String] = [
        "request_user_input": "AskUserQuestion",
        "AskUserQuestion": "AskUserQuestion",
        "ExitPlanMode": "ExitPlanMode",
    ]

    private static func attentionHookFromPreToolUse(_ payload: [String: Any]) -> String? {
        if let tool = payload["tool_name"] as? String,
           let mapped = attentionToolNames[tool] {
            return mapped
        }
        if let requestId = payload["_opencode_request_id"] as? String,
           requestId.contains("-PermissionRequest-") {
            return "PermissionRequest"
        }
        return nil
    }

    var onEvent: ((CmuxNotification) -> Void)?

    private var process: Process?
    private var lineBuffer = Data()
    private var restartAttempt = 0
    private var shouldRun = false
    private var workspaceNames: [String: String] = [:]
    private var workspaceRefreshInFlight = false

    nonisolated static func cliPath() -> String? {
        let bundled = appPath + "/Contents/Resources/bin/cmux"
        if FileManager.default.isExecutableFile(atPath: bundled) { return bundled }
        for candidate in ["/usr/local/bin/cmux", "/opt/homebrew/bin/cmux"]
        where FileManager.default.isExecutableFile(atPath: candidate) {
            return candidate
        }
        return nil
    }

    nonisolated static var isAvailable: Bool { cliPath() != nil }

    static var appIcon: NSImage {
        NSWorkspace.shared.icon(forFile: appPath)
    }

    static func isCmuxFrontmost() -> Bool {
        NSWorkspace.shared.frontmostApplication?.bundleIdentifier == bundleIdentifier
    }

    // MARK: Lifecycle

    func start() {
        guard !shouldRun else { return }
        shouldRun = true
        restartAttempt = 0
        refreshWorkspaceNames()
        spawn()
    }

    func stop() {
        shouldRun = false
        process?.terminationHandler = nil
        process?.terminate()
        process = nil
        lineBuffer.removeAll()
    }

    private func spawn() {
        guard shouldRun, process == nil, let cli = Self.cliPath() else { return }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: cli)
        // No --after/--cursor-file: start live at the latest seq so old events
        // never replay as stale notifications on app launch.
        proc.arguments = ["events", "--reconnect", "--category", "agent"]
        var env = ProcessInfo.processInfo.environment
        env["CMUX_QUIET"] = "1"
        proc.environment = env

        let stdout = Pipe()
        proc.standardOutput = stdout
        proc.standardError = FileHandle.nullDevice

        // Blocking read loop on a background queue — readabilityHandler proved
        // unreliable inside the app bundle. EOF exits the loop; the termination
        // handler owns the restart.
        let readHandle = stdout.fileHandleForReading
        DispatchQueue.global(qos: .utility).async { [weak self] in
            while true {
                let data = readHandle.availableData
                if data.isEmpty { break }
                Task { @MainActor [weak self] in
                    self?.consume(data)
                }
            }
        }
        proc.terminationHandler = { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.handleTermination()
            }
        }

        do {
            try proc.run()
            process = proc
        } catch {
            process = nil
            scheduleRestart()
        }
    }

    private func handleTermination() {
        process = nil
        lineBuffer.removeAll()
        scheduleRestart()
    }

    /// `--reconnect` survives socket drops; this only covers process death
    /// (cmux quit / CLI crash). Backs off 10s → 60s.
    private func scheduleRestart() {
        guard shouldRun else { return }
        restartAttempt += 1
        let delay = min(60.0, 10.0 * Double(restartAttempt))
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            self?.spawn()
        }
    }

    // MARK: Stream parsing

    private func consume(_ data: Data) {
        lineBuffer.append(data)
        while let newline = lineBuffer.firstIndex(of: UInt8(ascii: "\n")) {
            let line = lineBuffer.prefix(upTo: newline)
            lineBuffer.removeSubrange(...newline)
            if !line.isEmpty { parseLine(Data(line)) }
        }
    }

    private func parseLine(_ data: Data) {
        guard
            let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            obj["type"] as? String == "event",
            let name = obj["name"] as? String,
            name.hasPrefix("agent.hook."),
            let payload = obj["payload"] as? [String: Any],
            payload["phase"] as? String == "received",
            let workspaceId = obj["workspace_id"] as? String
        else { return }

        // A live stream healthily emits events → connection is good again.
        restartAttempt = 0

        var hook = String(name.dropFirst("agent.hook.".count))
        // cmux's feed pipeline rewrites codex hooks (and injected claude ones)
        // to PreToolUse, erasing hook_event_name. The original intent survives
        // in tool_name (questions / plan approval) or in the synthesized
        // request id ("<session>-PermissionRequest-<tool>-<ts>"; genuine tool
        // calls carry "call_*" ids instead).
        if hook == "PreToolUse" {
            guard let mapped = Self.attentionHookFromPreToolUse(payload) else { return }
            hook = mapped
        }
        let kind: CmuxEventKind
        if Self.attentionHooks.contains(hook) {
            kind = .attention
        } else if Self.finishedHooks.contains(hook) {
            kind = .finished
        } else {
            return
        }

        let notification = CmuxNotification(
            id: UUID(),
            kind: kind,
            hookName: hook,
            workspaceId: workspaceId,
            surfaceId: obj["surface_id"] as? String,
            windowId: obj["window_id"] as? String,
            sessionId: payload["session_id"] as? String,
            workspaceName: workspaceNames[workspaceId],
            occurredAt: Date()
        )
        onEvent?(notification)
        if notification.workspaceName == nil {
            refreshWorkspaceNames()
        }
    }

    // MARK: Workspace names

    /// Names are redacted out of the event stream, so titles come from
    /// `cmux workspace list --json --id-format both`, per window, cached by
    /// workspace UUID (the plain list omits `id` entirely).
    private func refreshWorkspaceNames() {
        guard !workspaceRefreshInFlight, let cli = Self.cliPath() else { return }
        workspaceRefreshInFlight = true
        Task.detached { [weak self] in
            var names: [String: String] = [:]
            for window in Self.listWindows(cli) {
                guard let windowId = window["id"] as? String else { continue }
                let output = Self.runCLI(
                    cli,
                    ["workspace", "list", "--json", "--id-format", "both", "--window", windowId]
                )
                if let data = output.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let workspaces = obj["workspaces"] as? [[String: Any]] {
                    for ws in workspaces {
                        if let id = ws["id"] as? String, let title = ws["title"] as? String {
                            names[id] = Self.cleanWorkspaceTitle(title)
                        }
                    }
                }
            }
            let resolved = names
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.workspaceRefreshInFlight = false
                if !resolved.isEmpty { self.workspaceNames = resolved }
            }
        }
    }

    func workspaceName(for id: String) -> String? { workspaceNames[id] }

    /// cmux prefixes busy workspace titles with a braille spinner glyph
    /// (U+2800–U+28FF) — drop it so banners show the plain name.
    private nonisolated static func cleanWorkspaceTitle(_ title: String) -> String {
        var scalars = Substring(title).unicodeScalars
        while let first = scalars.first,
              (0x2800...0x28FF).contains(first.value) || first.properties.isWhitespace {
            scalars.removeFirst()
        }
        return String(scalars)
    }

    private nonisolated static func listWindows(_ cli: String) -> [[String: Any]] {
        let output = runCLI(cli, ["list-windows", "--json"])
        guard let data = output.data(using: .utf8),
              let windows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return [] }
        return windows
    }

    /// Workspace ids currently on screen (each window's selected workspace).
    /// Used to skip cues for the workspace the user is already looking at.
    nonisolated static func visibleWorkspaceIds() async -> Set<String> {
        guard let cli = cliPath() else { return [] }
        return await Task.detached {
            Set(listWindows(cli).compactMap { $0["selected_workspace_id"] as? String })
        }.value
    }

    // MARK: Focus routing

    /// Focus cmux on the workspace/tab that produced the notification.
    func focus(_ notification: CmuxNotification) {
        guard let cli = Self.cliPath() else {
            activateCmuxApp()
            return
        }
        let n = notification
        Task.detached {
            if let windowId = n.windowId {
                _ = Self.runCLI(cli, ["focus-window", "--window", windowId])
            }
            _ = Self.runCLI(cli, ["select-workspace", "--workspace", n.workspaceId])
            if let surfaceId = n.surfaceId {
                _ = Self.runCLI(cli, ["focus-panel", "--panel", surfaceId])
            }
            await MainActor.run { Self.staticActivateCmuxApp() }
        }
    }

    private func activateCmuxApp() { Self.staticActivateCmuxApp() }

    private static func staticActivateCmuxApp() {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleIdentifier) else { return }
        NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
    }

    private nonisolated static func runCLI(_ cli: String, _ args: [String]) -> String {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: cli)
        proc.arguments = args
        var env = ProcessInfo.processInfo.environment
        env["CMUX_QUIET"] = "1"
        proc.environment = env
        let stdout = Pipe()
        proc.standardOutput = stdout
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
        } catch {
            return ""
        }
        let data = stdout.fileHandleForReading.readDataToEndOfFile()
        proc.waitUntilExit()
        return String(data: data, encoding: .utf8) ?? ""
    }
}
