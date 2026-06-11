import Foundation

@MainActor
final class DaemonLauncher {
    private var process: Process?
    private var stopping = false
    private var restartCount = 0
    private var windowStart: Date = .distantPast

    func launch() {
        guard let url = findBinary() else {
            print("[bagentd] binary not found — run `cargo build` first")
            return
        }
        startProcess(url: url)
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
