import Foundation

@MainActor
final class DaemonLauncher {
    private var process: Process?
    private var stopping = false
    private var restartCount = 0
    private var windowStart: Date = .distantPast
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
        startProcess(url: url)
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

    func stop() {
        stopping = true
        process?.terminate()
        process = nil
    }

    // MARK: - Private

    private func startProcess(url: URL) {
        let p = Process()
        p.executableURL = url
        var environment = ProcessInfo.processInfo.environment
        environment["BAGENT_DEFAULT_MODEL"] = UserDefaults.standard.string(forKey: "bagent.model") ?? "qwen2.5:7b"
        environment["BAGENT_CLASSIFIER_MODEL"] = UserDefaults.standard.string(forKey: "bagent.classifier_model") ?? "qwen3:0.6b"
        environment["BAGENT_VISION_MODEL"] = "qwen2.5vl:7b"
        p.environment = environment
        // terminationHandler is called on an arbitrary thread; dispatch back to @MainActor.
        p.terminationHandler = { proc in
            // Normal exit (our own stop()) — don't restart.
            guard proc.terminationStatus != 0 || proc.terminationReason == .uncaughtSignal else { return }
            Task { @MainActor [weak self] in
                self?.handleCrash(url: url)
            }
        }
        do {
            try p.run()
            process = p
            print("[bagentd] started pid \(p.processIdentifier)")
        } catch {
            print("[bagentd] failed to start: \(error)")
        }
    }

    private func handleCrash(url: URL) {
        guard !stopping else { return }

        let now = Date()
        if now.timeIntervalSince(windowStart) > 60 {
            restartCount = 0
            windowStart = now
        }
        restartCount += 1

        guard restartCount <= 3 else {
            print("[bagentd] ≥3 crashes/min — giving up")
            return
        }
        print("[bagentd] crashed — restart \(restartCount)/3")

        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(500))
            self.startProcess(url: url)
        }
    }

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
