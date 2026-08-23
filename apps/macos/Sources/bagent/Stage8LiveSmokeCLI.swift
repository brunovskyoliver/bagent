import AppKit
import SwiftUI

#if BAGENT_ACCEPTANCE
/// Signed-candidate observation and mutation boundary for the A59 disposable
/// live smoke. It uses the production client and projection, while writing
/// only counts, enum values, and identities' structural relationships.
@MainActor
enum Stage8LiveSmokeCLI {
    static let environmentKey = "BAGENT_STAGE8_ACCEPTANCE_FIXTURES"

    static func runProjection(outputURL: URL) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            let client = DaemonClient()
            let viewModel = ChatViewModel(startMonitoring: false)
            var observedStages = Set<StageRailStage>()
            var renderedBytes = 0
            var renderCount = 0
            var lastSnapshot: NotchWorkSnapshot?
            let deadline = ContinuousClock.now.advanced(by: .seconds(12))
            while ContinuousClock.now < deadline,
                  !observedStages.isSuperset(of: [.think, .tool]) {
                let snapshot = try await client.fetchAuthoritativeSnapshot()
                lastSnapshot = snapshot
                try viewModel.applyAuthoritativeSnapshot(snapshot)
                let presentation = viewModel.notchPresentation
                if let stage = presentation.rail.selectedStage,
                   [.think, .tool].contains(stage),
                   !observedStages.contains(stage) {
                    guard presentation.interactionMode == .collapsed else {
                        throw LiveSmokeError.assertion(
                            "signed activity rail was not in collapsed presentation"
                        )
                    }
                    let png = try renderCollapsedRail(presentation)
                    try png.write(
                        to: outputURL.deletingLastPathComponent()
                            .appendingPathComponent("collapsed-\(stage.rawValue).png"),
                        options: .atomic
                    )
                    renderedBytes += png.count
                    renderCount += 1
                    observedStages.insert(stage)
                }
                try await Task.sleep(for: .milliseconds(20))
            }
            guard let snapshot = lastSnapshot else {
                throw LiveSmokeError.assertion("signed UI did not fetch an authoritative snapshot")
            }
            let presentation = viewModel.notchPresentation
            guard observedStages.isSuperset(of: [.think, .tool]),
                  renderCount == 2,
                  renderedBytes > 2_000 else {
                throw LiveSmokeError.assertion(
                    "signed UI did not render both collapsed Think and Tool presentations"
                )
            }
            try writeEvidence([
                "status": "pass",
                "signed_ui_pid": ProcessInfo.processInfo.processIdentifier,
                "snapshot_cursor": snapshot.cursor,
                "observed_stage": presentation.rail.selectedStage?.rawValue ?? "none",
                "status_pill": presentation.statusPill.label ?? "hidden",
                "active_automation_count": presentation.activeAutomationCount,
                "foreground_work": presentation.hasActiveForegroundWork,
                "collapsed_presentation_observed": true,
                "collapsed_render_count": renderCount,
                "collapsed_render_bytes": renderedBytes,
                "thinking_stage_observed": observedStages.contains(.think),
                "tool_stage_observed": observedStages.contains(.tool),
                "status_pill_anchor_invariant": NotchPillLayout.origin(maxPanelWidth: 741)
                    == CGPoint(x: 643, y: 9),
            ], to: outputURL)
            return 0
        } catch {
            try? writeEvidence([
                "status": "failed",
                "signed_ui_pid": ProcessInfo.processInfo.processIdentifier,
                "error": String(describing: error),
            ], to: outputURL)
            return 1
        }
    }

    static func run(
        runIdentity: String,
        workIdentity: String,
        workRevision: UInt64,
        outputURL: URL
    ) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            let client = DaemonClient()
            let beforeSnapshot = try await client.fetchAuthoritativeSnapshot()
            guard let work = beforeSnapshot.works.first(where: { $0.identity == workIdentity }),
                  work.revision == workRevision else {
                throw LiveSmokeError.assertion("signed UI could not observe the selected Work revision")
            }

            let viewModel = ChatViewModel(startMonitoring: false)
            try viewModel.applyAuthoritativeSnapshot(beforeSnapshot)
            let presentation = viewModel.notchPresentation
            guard presentation.revision.cursor == beforeSnapshot.cursor,
                  presentation.snapshot.works.contains(where: { $0.identity == workIdentity }),
                  presentation.rail.selectedStage != nil else {
                throw LiveSmokeError.assertion("signed UI projection did not admit the selected Work")
            }

            let sessionIdentity = "automation-session:\(runIdentity)"
            let session = try await client.automationSession(identity: sessionIdentity)
            guard session.finalOutputAvailable || session.resultSummary != nil,
                  !session.activityTimeline.isEmpty else {
                throw LiveSmokeError.assertion("signed UI result projection is incomplete")
            }

            try await client.openAutomationSession(
                identity: sessionIdentity,
                commandIdentity: "stage8-signed-open-\(runIdentity)",
                expectedRevision: workRevision
            )
            let opened = try await client.automationSession(identity: sessionIdentity)
            guard opened.attention == "viewed" else {
                throw LiveSmokeError.assertion("signed UI result did not become viewed")
            }

            let currentBeforeContinuation = try await client.currentChat()
            let continuation = try await client.continueAutomationSession(
                identity: sessionIdentity,
                seed: "Stage 8 disposable signed continuation",
                confirmedReplacement: true,
                commandIdentity: "stage8-signed-continue-\(runIdentity)"
            )
            guard continuation.targetCurrentChatIdentity != currentBeforeContinuation.identity else {
                throw LiveSmokeError.assertion("signed UI continuation did not create a new Current Chat")
            }

            let currentBeforeClear = try await client.currentChat()
            let cleared = try await client.clearCurrentChat(
                identity: currentBeforeClear.identity,
                expectedRevision: currentBeforeClear.revision,
                commandIdentity: "stage8-signed-clear-current-chat",
                confirmedNonEmpty: true
            )
            guard cleared.turnCount == 0,
                  cleared.draft == nil,
                  cleared.identity != continuation.targetCurrentChatIdentity else {
                throw LiveSmokeError.assertion("signed UI /clear did not rotate only the current chat")
            }

            let permission = await client.fullDiskAccessProbe()
            guard permission.mail == .granted, permission.notes == .granted else {
                throw LiveSmokeError.assertion("signed UI permission reread was not granted")
            }

            try writeEvidence([
                "status": "pass",
                "signed_ui_pid": ProcessInfo.processInfo.processIdentifier,
                "observed_cursor": beforeSnapshot.cursor,
                "observed_stage": presentation.rail.selectedStage?.rawValue ?? "none",
                "observed_status": presentation.statusPill.label ?? "hidden",
                "activity_count": session.activityTimeline.count,
                "result_opened": true,
                "continuation_target_changed": true,
                "clear_scoped": true,
                "cleared_turn_count": cleared.turnCount,
                "cleared_draft": cleared.draft != nil,
                "permission_reread": true,
                "tcc_mutated": false,
            ], to: outputURL)
            return 0
        } catch {
            try? writeEvidence([
                "status": "failed",
                "signed_ui_pid": ProcessInfo.processInfo.processIdentifier,
                "error": String(describing: error),
            ], to: outputURL)
            fputs("Stage 8 signed live observation failed: \(error)\n", stderr)
            return 1
        }
    }

    private enum LiveSmokeError: Error, CustomStringConvertible {
        case assertion(String)

        var description: String {
            switch self {
            case .assertion(let message): message
            }
        }
    }

    private static func renderCollapsedRail(_ presentation: NotchPresentation) throws -> Data {
        let host = NSHostingView(
            rootView: ZStack {
                Color.black
                ActivityPeekStageRailView(presentation: presentation, action: {})
                    .padding(16)
            }
            .frame(width: 460, height: 108)
        )
        host.frame = CGRect(x: 0, y: 0, width: 460, height: 108)
        host.layoutSubtreeIfNeeded()
        guard !presentation.rail.accessibilityValue.isEmpty,
              let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else {
            throw LiveSmokeError.assertion("collapsed activity rail was not inspectable")
        }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]), png.count > 1_000 else {
            throw LiveSmokeError.assertion("collapsed activity rail render was empty")
        }
        return png
    }

    private static func writeEvidence(_ object: [String: Any], to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }
}
#endif
