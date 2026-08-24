import Foundation
import Darwin

/// Builds the LaunchAgent plist for the daemon. Pure — unit-testable.
enum DaemonLaunchAgent {
    static let label = "com.bagent.daemon"

    private static func xmlEscaped(_ value: String) -> String {
        value
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "'", with: "&apos;")
    }

    static func runtimeEnvironment(processEnvironment: [String: String]) -> [String: String] {
        var environment = [
            "BAGENT_BASERT_BASE_URL": "http://127.0.0.1:8082/v1",
            "BAGENT_BASERT_API_KEY": ModelRuntimeConfiguration.apiKey,
            "BAGENT_BASERT_LOG_PATH": ModelRuntimeConfiguration.logURL.path,
            "BAGENT_DEFAULT_MODEL": ModelRuntimeConfiguration.model,
            "BAGENT_CLASSIFIER_MODEL": ModelRuntimeConfiguration.model,
            "BAGENT_CHAT_MODEL_PATH":
                ModelRuntimeConfiguration.cachedModelURL(ModelRuntimeConfiguration.model).path,
            "BAGENT_SYNTHESIS_MODEL_PATH":
                ModelRuntimeConfiguration.cachedModelURL(ModelRuntimeConfiguration.synthesisModel).path,
        ]
        // Typed evidence routing is unconditional; the resolver mode is an
        // independent local setting and is forwarded verbatim when present.
        if let resolverMode = processEnvironment["BAGENT_REFERENCE_RESOLVER_MODE"] {
            environment["BAGENT_REFERENCE_RESOLVER_MODE"] = resolverMode
        }
        if processEnvironment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"] == "1" {
            environment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"] = "1"
        }
        return environment
    }

    static var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/bagent/daemon.log")
    }

    /// launchd property list. KeepAlive on crash only: a clean exit (explicit
    /// shutdown, `launchctl bootout`) stays down; a crash is restarted by
    /// launchd with its default throttle.
    static func plistContent(binaryPath: String, environment: [String: String]) -> String {
        let envEntries = environment
            .sorted { $0.key < $1.key }
            .map {
                "        <key>\(xmlEscaped($0.key))</key>\n        <string>\(xmlEscaped($0.value))</string>"
            }
            .joined(separator: "\n")
        return """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>\(label)</string>
            <key>ProgramArguments</key>
            <array>
                <string>\(xmlEscaped(binaryPath))</string>
            </array>
            <key>EnvironmentVariables</key>
            <dict>
        \(envEntries)
            </dict>
            <key>RunAtLoad</key>
            <true/>
            <key>KeepAlive</key>
            <dict>
                <key>SuccessfulExit</key>
                <false/>
            </dict>
            <key>StandardOutPath</key>
            <string>\(xmlEscaped(logURL.path))</string>
            <key>StandardErrorPath</key>
            <string>\(xmlEscaped(logURL.path))</string>
        </dict>
        </plist>
        """
    }
}

/// Read-only inputs for the daemon-owned Model Runtime. Swift never starts,
/// probes, retires, or restarts the dedicated port-8082 service.
enum ModelRuntimeConfiguration {
    static let model = "basecompute/Qwen3-4B-Instruct-2507"
    static let synthesisModel = "basecompute/Qwen3.6-35B-A3B"
    static let apiKey = "basert-local"

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/bagent/basert.log")
    }

    static func cachedModelURL(_ modelID: String) -> URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Caches/baseRT/models")
            .appendingPathComponent(modelID)
            .appendingPathComponent("default-q4/model.base")
    }

}

/// Manages daemon residency via a per-user launchd agent.
///
/// Lifecycle contract:
/// - App launch reinstalls the LaunchAgent (current binary path + model env)
///   and restarts the daemon — deterministic upgrade/packaging behavior.
/// - App exit leaves the daemon running so scheduled automations continue.
/// - Crashes are restarted by launchd (`KeepAlive.SuccessfulExit=false`).
/// - Explicit shutdown is `shutdownDaemon()` (launchctl bootout).
/// - Port/token discovery is unchanged: the daemon itself writes
///   `daemon.port` / `daemon.token` / `daemon.pid` in Application Support.
@MainActor
final class DaemonLauncher {
    func launch() {
        guard let url = findBinary() else {
            print("[bagentd] binary not found — run `cargo build` first")
            return
        }
        terminateLegacyChildDaemon()
        Task {
            await installAndStart(binary: url)
        }
    }

    /// Intentionally does not stop the daemon: scheduled automations keep
    /// running after the notch app exits.
    func stop() {}

    /// Explicit shutdown — unloads the agent; launchd will not restart it.
    func shutdownDaemon() {
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
    }

    // MARK: - launchd

    private func installAndStart(binary: URL) async {
        let processEnvironment = ProcessInfo.processInfo.environment
        let env = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: processEnvironment)
        let plist = DaemonLaunchAgent.plistContent(binaryPath: binary.path, environment: env)
        let plistURL = DaemonLaunchAgent.plistURL
        do {
            try FileManager.default.createDirectory(
                at: plistURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.createDirectory(
                at: DaemonLaunchAgent.logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try plist.write(to: plistURL, atomically: true, encoding: .utf8)
        } catch {
            print("[bagentd] failed to write LaunchAgent: \(error)")
            return
        }
        // Deterministic restart into the current binary + env on every app
        // launch (same semantics the app had when it owned the process).
        let previousPID = launchdManagedPid()
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
        if let previousPID {
            for _ in 0..<60 {
                if kill(previousPID, 0) != 0 { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
        let status = await bootstrapLaunchAgent(plistURL: plistURL)
        print(status == 0
            ? "[bagentd] launchd agent bootstrapped"
            : "[bagentd] launchctl bootstrap failed (\(status))")
    }

    @discardableResult
    private func runLaunchctl(_ args: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = args
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do {
            try p.run()
            p.waitUntilExit()
            return p.terminationStatus
        } catch {
            print("[bagentd] launchctl \(args.first ?? "") failed: \(error)")
            return -1
        }
    }

    private func bootstrapLaunchAgent(plistURL: URL) async -> Int32 {
        var status: Int32 = -1
        for attempt in 1...3 {
            status = runLaunchctl(["bootstrap", "gui/\(getuid())", plistURL.path])
            if status == 0 { return status }
            if attempt < 3 {
                try? await Task.sleep(for: .milliseconds(750))
            }
        }
        return status
    }

    /// Pre-launchd app versions spawned bagentd as a child and recorded its
    /// pid. Kill such a daemon once so launchd becomes the single owner.
    private func terminateLegacyChildDaemon() {
        let pidURL = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first!
            .appendingPathComponent("bagent")
            .appendingPathComponent("daemon.pid")
        guard let raw = try? String(contentsOf: pidURL, encoding: .utf8),
              let pid = Int32(raw.trimmingCharacters(in: .whitespacesAndNewlines)),
              pid > 0 else { return }
        // Only kill daemons that are NOT launchd-managed (launchd-managed pids
        // are listed under our label).
        if launchdManagedPid() == pid { return }
        if kill(pid, 0) == 0 {
            print("[bagentd] terminating legacy child daemon pid \(pid)")
            kill(pid, SIGTERM)
            Thread.sleep(forTimeInterval: 0.25)
        }
        try? FileManager.default.removeItem(at: pidURL)
    }

    /// Current daemon pid according to launchd, or nil when not running.
    private func launchdManagedPid() -> Int32? {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = ["print", "gui/\(getuid())/\(DaemonLaunchAgent.label)"]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        guard p.terminationStatus == 0,
              let out = String(data: data, encoding: .utf8) else { return nil }
        for line in out.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("pid = "), let pid = Int32(trimmed.dropFirst(6)) {
                return pid
            }
        }
        return nil
    }

    // MARK: - Binary discovery

    private func findBinary() -> URL? {
        if let execURL = Bundle.main.executableURL {
            let bundled = execURL.deletingLastPathComponent().appendingPathComponent("bagentd")
            if FileManager.default.fileExists(atPath: bundled.path) { return bundled }
        }
        let dev = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("target/debug/bagentd")
        if FileManager.default.fileExists(atPath: dev.path) { return dev }
        return nil
    }
}
