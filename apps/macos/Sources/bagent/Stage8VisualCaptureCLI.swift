import AppKit
import SwiftUI

@MainActor
enum Stage8VisualCaptureCLI {
    static let environmentKey = "BAGENT_STAGE8_VISUAL_FIXTURE"

    static func run(outputDirectory: URL, evidenceURL: URL) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            try FileManager.default.createDirectory(at: outputDirectory, withIntermediateDirectories: true)
            let fixtures = try stateFixtures()
            for fixture in fixtures {
                let view = NSHostingView(rootView: AnyView(catalogView(fixture.presentation)))
                view.frame = NSRect(x: 0, y: 0, width: 741, height: 312)
                view.layoutSubtreeIfNeeded()
                guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
                    throw CaptureError.render(fixture.name)
                }
                view.cacheDisplay(in: view.bounds, to: bitmap)
                guard let png = bitmap.representation(using: .png, properties: [:]), png.count > 1_000 else {
                    throw CaptureError.render(fixture.name)
                }
                try png.write(to: outputDirectory.appendingPathComponent("\(fixture.name).png"))
            }
            let captures = try FileManager.default.contentsOfDirectory(at: outputDirectory, includingPropertiesForKeys: nil)
                .filter { $0.pathExtension == "png" }
            guard captures.count == fixtures.count else { throw CaptureError.count }

            let transitionEvidence = try await recordTransitions(
                fixtures: fixtures,
                outputDirectory: outputDirectory.appendingPathComponent("transition-frames", isDirectory: true)
            )
            let evidence: [String: Any] = [
                "status": "pass",
                "signed_process_id": ProcessInfo.processInfo.processIdentifier,
                "rendered_notch_state_count": captures.count,
                "transition_count": transitionEvidence.transitionCount,
                "recorded_transition_frame_count": transitionEvidence.frameCount,
                "distinct_transition_frame_count": transitionEvidence.distinctFrameCount,
                "normal_motion_frame_count": transitionEvidence.normalFrameCount,
                "reduced_motion_frame_count": transitionEvidence.reducedFrameCount,
                "interruptions_injected": transitionEvidence.interruptionsInjected,
                "interruptions_reconciled": transitionEvidence.interruptionsReconciled,
                "status_pill_anchor_invariant": NotchPillLayout.origin(maxPanelWidth: 741) == CGPoint(x: 643, y: 9),
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: evidenceURL, options: .atomic)
            return 0
        } catch {
            fputs("Stage 8 signed visual capture failed: \(error)\n", stderr)
            return 1
        }
    }

    private enum CaptureError: Error { case render(String), count, transition }

    private struct TransitionEvidence {
        let transitionCount: Int
        let frameCount: Int
        let distinctFrameCount: Int
        let normalFrameCount: Int
        let reducedFrameCount: Int
        let interruptionsInjected: Int
        let interruptionsReconciled: Int
    }

    private static func recordTransitions(
        fixtures: [(name: String, presentation: NotchPresentation)],
        outputDirectory: URL
    ) async throws -> TransitionEvidence {
        try FileManager.default.createDirectory(at: outputDirectory, withIntermediateDirectories: true)
        var transitionCount = 0
        var frameCount = 0
        var normalFrameCount = 0
        var reducedFrameCount = 0
        var interruptionsInjected = 0
        var interruptionsReconciled = 0
        var frameData = Set<Data>()

        for reduceMotion in [false, true] {
            let mode = reduceMotion ? "reduced" : "normal"
            let modeDirectory = outputDirectory.appendingPathComponent(mode, isDirectory: true)
            try FileManager.default.createDirectory(at: modeDirectory, withIntermediateDirectories: true)
            let viewModel = ChatViewModel(startMonitoring: false)
            viewModel.setNotchReduceMotion(reduceMotion)
            let host = NSHostingView(
                rootView: NotchWrapView(
                    notchWidth: 221,
                    notchHeight: 38,
                    viewModel: viewModel,
                    onTap: {},
                    onHoverChanged: { _ in },
                    acceptanceReduceMotionOverride: reduceMotion
                )
            )
            host.frame = CGRect(x: 0, y: 0, width: 741, height: 312)
            let panel = NSPanel(
                contentRect: CGRect(x: -10_000, y: -10_000, width: 741, height: 312),
                styleMask: [.borderless],
                backing: .buffered,
                defer: false
            )
            panel.isReleasedWhenClosed = false
            panel.contentView = host
            panel.orderFront(nil)
            defer { panel.close() }

            for (index, fixture) in fixtures.enumerated() {
                try viewModel.applyAuthoritativeSnapshot(fixture.presentation.snapshot)
                transitionCount += 1
                let delays: [Duration] = reduceMotion
                    ? [.zero, .milliseconds(30), .milliseconds(70)]
                    : [.zero, .milliseconds(120), .milliseconds(650)]
                for (frameIndex, delay) in delays.enumerated() {
                    if delay != .zero { try await Task.sleep(for: delay) }
                    host.layoutSubtreeIfNeeded()
                    let data = try capturePNG(host, name: "\(mode)-\(fixture.name)-\(frameIndex)")
                    try data.write(to: modeDirectory.appendingPathComponent(
                        String(format: "%02d-%@-%d.png", index, fixture.name, frameIndex)
                    ))
                    frameData.insert(data)
                    frameCount += 1
                    if reduceMotion { reducedFrameCount += 1 } else { normalFrameCount += 1 }
                }
            }

            let loading = NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: 12,
                daemonGeneration: "signed-transition-recording",
                works: [work("interrupted-live", state: .waitingForModel)],
                pendingApprovals: [],
                model: .loading
            )
            let failed = NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: 13,
                daemonGeneration: "signed-transition-recording",
                works: [work("interrupted-live", state: .failed, attention: .failed)],
                pendingApprovals: [],
                model: .ready
            )
            try viewModel.applyAuthoritativeSnapshot(loading)
            try await Task.sleep(for: reduceMotion ? .milliseconds(20) : .milliseconds(120))
            let beforeInterruption = try capturePNG(host, name: "\(mode)-interruption-before")
            try beforeInterruption.write(to: modeDirectory.appendingPathComponent("interruption-before.png"))
            try viewModel.applyAuthoritativeSnapshot(failed)
            interruptionsInjected += 1
            let afterInterruption = try capturePNG(host, name: "\(mode)-interruption-injected")
            try afterInterruption.write(to: modeDirectory.appendingPathComponent("interruption-injected.png"))
            try await Task.sleep(for: reduceMotion ? .milliseconds(70) : .milliseconds(700))
            let settledInterruption = try capturePNG(host, name: "\(mode)-interruption-settled")
            try settledInterruption.write(to: modeDirectory.appendingPathComponent("interruption-settled.png"))
            let expectedFailed = try await captureSettledHistory(
                fixtures.map(\.presentation.snapshot) + [loading, failed],
                reduceMotion: reduceMotion,
                name: "\(mode)-expected-failed"
            )
            try expectedFailed.write(to: modeDirectory.appendingPathComponent("interruption-expected-failed.png"))
            for data in [beforeInterruption, afterInterruption, settledInterruption, expectedFailed] {
                frameData.insert(data)
                frameCount += 1
                if reduceMotion { reducedFrameCount += 1 } else { normalFrameCount += 1 }
            }
            guard viewModel.notchPresentation.snapshot.works.first?.state == .failed,
                  viewModel.notchPresentation.revision.cursor == failed.cursor,
                  settledInterruption == expectedFailed else {
                throw CaptureError.transition
            }
            interruptionsReconciled += 1
        }

        guard transitionCount == fixtures.count * 2,
              interruptionsInjected == 2,
              interruptionsReconciled == interruptionsInjected,
              frameData.count > fixtures.count else {
            throw CaptureError.transition
        }
        return TransitionEvidence(
            transitionCount: transitionCount,
            frameCount: frameCount,
            distinctFrameCount: frameData.count,
            normalFrameCount: normalFrameCount,
            reducedFrameCount: reducedFrameCount,
            interruptionsInjected: interruptionsInjected,
            interruptionsReconciled: interruptionsReconciled
        )
    }

    private static func capturePNG(_ view: NSView, name: String) throws -> Data {
        view.layoutSubtreeIfNeeded()
        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
            throw CaptureError.render(name)
        }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]), png.count > 1_000 else {
            throw CaptureError.render(name)
        }
        return png
    }

    private static func captureSettledHistory(
        _ snapshots: [NotchWorkSnapshot],
        reduceMotion: Bool,
        name: String
    ) async throws -> Data {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.setNotchReduceMotion(reduceMotion)
        let host = NSHostingView(
            rootView: NotchWrapView(
                notchWidth: 221,
                notchHeight: 38,
                viewModel: viewModel,
                onTap: {},
                onHoverChanged: { _ in },
                acceptanceReduceMotionOverride: reduceMotion
            )
        )
        host.frame = CGRect(x: 0, y: 0, width: 741, height: 312)
        let panel = NSPanel(
            contentRect: CGRect(x: -10_000, y: -10_000, width: 741, height: 312),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        panel.isReleasedWhenClosed = false
        panel.contentView = host
        panel.orderFront(nil)
        defer { panel.close() }
        for snapshot in snapshots {
            try viewModel.applyAuthoritativeSnapshot(snapshot)
            host.layoutSubtreeIfNeeded()
        }
        try await Task.sleep(for: reduceMotion ? .milliseconds(300) : .milliseconds(700))
        return try capturePNG(host, name: name)
    }

    private static func catalogView(_ presentation: NotchPresentation) -> some View {
        ZStack(alignment: .topLeading) {
            Color(red: 0.13, green: 0.14, blue: 0.16)
            NotchWrapShape(
                wingWidth: presentation.geometry.wingWidth,
                bridgeHeight: presentation.geometry.bridgeHeight,
                notchOffset: 260,
                notchWidth: 221,
                notchHeight: 32,
                cornerRadius: 10
            ).fill(Color.black)
            if presentation.geometry.bridgeHeight > 0 {
                ActivityPeekStageRailView(presentation: presentation, action: {})
                    .frame(width: 410, height: max(48, presentation.geometry.bridgeHeight - 12))
                    .position(x: 370.5, y: 32 + presentation.geometry.bridgeHeight / 2)
            }
            if presentation.statusPill.label != nil {
                InvariantNotchStatusPill(
                    presentation: presentation.statusPill,
                    activeAutomationCount: presentation.activeAutomationCount,
                    action: {}
                ).position(
                    x: NotchPillLayout.origin(maxPanelWidth: 741).x + 37,
                    y: NotchPillLayout.origin(maxPanelWidth: 741).y + 9
                )
            }
        }.frame(width: 741, height: 312)
    }

    private static func stateFixtures() throws -> [(name: String, presentation: NotchPresentation)] {
        let states: [(String, [NotchWork], [NotchApproval], NotchModelPhase)] = [
            ("idle", [], [], .unloaded),
            ("queued", [work("queued", state: .queued)], [], .ready),
            ("loading", [work("loading", state: .waitingForModel)], [], .loading),
            ("thinking", [work("thinking", state: .running, origin: .conversation)], [], .ready),
            ("tool", [work("tool", state: .running, activity: .web)], [], .ready),
            ("approval", [work("approval", state: .waitingForApproval)], [.init(identity: "approval-a", workIdentity: "approval")], .ready),
            ("streaming", [work("streaming", state: .running, origin: .conversation, activity: .chat)], [], .ready),
            ("completion", [work("completion", state: .completed, attention: .unread)], [], .ready),
            ("failure", [work("failure", state: .failed, attention: .failed)], [], .ready),
            ("cancellation", [work("cancellation", state: .cancelled)], [], .unloaded),
            ("interruption", [work("interruption", state: .abandoned)], [], .unloaded),
        ]
        return try states.enumerated().map { index, state in
            let snapshot = NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: UInt64(index + 1),
                daemonGeneration: "signed-state-catalog",
                works: state.1,
                pendingApprovals: state.2,
                model: state.3
            )
            return (state.0, try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot), reduceMotion: true))
        }
    }

    private static func work(
        _ identity: String,
        state: NotchWorkState,
        origin: NotchWorkOrigin = .automation,
        activity: NotchActivityCategory? = nil,
        attention: NotchTerminalAttention? = nil
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 1,
            origin: origin,
            state: state,
            activity: activity.map(NotchActivity.init(category:)),
            queuePosition: state == .queued ? 1 : nil,
            automationDisplayName: origin == .automation ? "Saved Automation" : nil,
            terminalAttention: attention,
            claimedOrder: 1
        )
    }
}
