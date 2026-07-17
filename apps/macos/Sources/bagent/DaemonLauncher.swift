import Foundation
import Darwin

/// Builds the LaunchAgent plist for the daemon. Pure — unit-testable.
enum DaemonLaunchAgent {
    static let label = "com.bagent.daemon"

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
            .map { "        <key>\($0.key)</key>\n        <string>\($0.value)</string>" }
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
                <string>\(binaryPath)</string>
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
            <string>\(logURL.path)</string>
            <key>StandardErrorPath</key>
            <string>\(logURL.path)</string>
        </dict>
        </plist>
        """
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
    /// Handle to the Ollama process we may have spawned. Kept alive so we don't
    /// spawn a second instance if `launch()` is called more than once.
    private var ollamaProcess: Process?

    func launch() {
        // Ensure Ollama is running before the daemon tries to call it.
        Task { await ensureOllamaRunning() }

        guard let url = findBinary() else {
            print("[bagentd] binary not found — run `cargo build` first")
            return
        }
        terminateLegacyChildDaemon()
        installAndStart(binary: url)
    }

    /// Intentionally does not stop the daemon: scheduled automations keep
    /// running after the notch app exits.
    func stop() {}

    /// Explicit shutdown — unloads the agent; launchd will not restart it.
    func shutdownDaemon() {
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
    }

    // MARK: - launchd

    private func installAndStart(binary: URL) {
        let env = [
            "BAGENT_DEFAULT_MODEL": UserDefaults.standard.string(forKey: "bagent.model") ?? "qwen3:8b",
            "BAGENT_CLASSIFIER_MODEL": UserDefaults.standard.string(forKey: "bagent.classifier_model") ?? "qwen3:0.6b",
            "BAGENT_VISION_MODEL": "qwen2.5vl:7b",
        ]
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
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
        let status = runLaunchctl(["bootstrap", "gui/\(getuid())", plistURL.path])
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

    // MARK: - Ollama autostart

    /// Checks whether Ollama is reachable; spawns `ollama serve` if not,
    /// then polls until it answers or the timeout elapses.
    private func ensureOllamaRunning() async {
        guard await !isOllamaUp() else {
            print("[ollama] already running")
            return
        }
        guard let ollamaBin = findOllamaBinary() else {
            print("[ollama] binary not found — install from https://ollama.com")
            return
        }

        // Spawn `ollama serve` in the background; discard stdout/stderr.
        let p = Process()
        p.executableURL = ollamaBin
        p.arguments = ["serve"]
        p.standardOutput = FileHandle.nullDevice
        p.standardError  = FileHandle.nullDevice
        do {
            try p.run()
            ollamaProcess = p
            print("[ollama] started pid \(p.processIdentifier)")
        } catch {
            print("[ollama] failed to start: \(error)")
            return
        }

        // Poll up to 6 s (12 × 0.5 s) for the HTTP API to answer.
        for attempt in 1...12 {
            try? await Task.sleep(for: .milliseconds(500))
            if await isOllamaUp() {
                print("[ollama] ready after \(attempt) poll(s)")
                return
            }
        }
        print("[ollama] did not become ready in time — continuing anyway")
    }

    /// Async HTTP probe — returns true if Ollama's `/api/tags` responds 200.
    private func isOllamaUp() async -> Bool {
        guard let url = URL(string: "http://127.0.0.1:11434/api/tags") else { return false }
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 1)
        req.httpMethod = "GET"
        guard let (_, response) = try? await URLSession.shared.data(for: req) else { return false }
        return (response as? HTTPURLResponse)?.statusCode == 200
    }

    /// Finds the `ollama` binary in well-known locations.
    private func findOllamaBinary() -> URL? {
        let candidates = [
            "/usr/local/bin/ollama",
            "/opt/homebrew/bin/ollama",
            "/usr/bin/ollama",
        ]
        for path in candidates {
            let url = URL(fileURLWithPath: path)
            if FileManager.default.isExecutableFile(atPath: url.path) { return url }
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
