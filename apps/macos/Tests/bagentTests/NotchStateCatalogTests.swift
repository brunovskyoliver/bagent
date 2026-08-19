import AppKit
import SwiftUI
import XCTest
@testable import bagent

@MainActor
final class NotchStateCatalogTests: XCTestCase {
    func testRendersAcceptedStateCatalogWithoutClipping() throws {
        let outputDirectory = URL(fileURLWithPath:
            ProcessInfo.processInfo.environment["BAGENT_NOTCH_CAPTURE_DIR"]
                ?? FileManager.default.temporaryDirectory
                    .appendingPathComponent("bagent-notch-state-catalog")
                    .path
        )
        try FileManager.default.createDirectory(
            at: outputDirectory,
            withIntermediateDirectories: true
        )

        for fixture in try fixtures() {
            let view = NSHostingView(rootView: AnyView(catalogView(fixture.presentation)))
            view.frame = NSRect(x: 0, y: 0, width: 741, height: 312)
            view.layoutSubtreeIfNeeded()
            guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
                XCTFail("could not allocate bitmap for \(fixture.name)")
                continue
            }
            view.cacheDisplay(in: view.bounds, to: bitmap)
            guard let png = bitmap.representation(using: .png, properties: [:]) else {
                XCTFail("could not encode \(fixture.name)")
                continue
            }
            XCTAssertGreaterThan(png.count, 1_000, "empty capture for \(fixture.name)")
            try png.write(to: outputDirectory.appendingPathComponent("\(fixture.name).png"))
        }

        let captures = try FileManager.default.contentsOfDirectory(
            at: outputDirectory,
            includingPropertiesForKeys: nil
        ).filter { $0.pathExtension == "png" && $0.lastPathComponent != "contact-sheet.png" }
        XCTAssertEqual(captures.count, 11)
    }

    private func catalogView(_ presentation: NotchPresentation) -> some View {
        ZStack(alignment: .topLeading) {
            Color(red: 0.13, green: 0.14, blue: 0.16)
            NotchWrapShape(
                wingWidth: presentation.geometry.wingWidth,
                bridgeHeight: presentation.geometry.bridgeHeight,
                notchOffset: 260,
                notchWidth: 221,
                notchHeight: 32,
                cornerRadius: 10
            )
            .fill(Color.black)
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
                )
                .position(
                    x: NotchPillLayout.origin(maxPanelWidth: 741).x + 37,
                    y: NotchPillLayout.origin(maxPanelWidth: 741).y + 9
                )
            }
        }
        .frame(width: 741, height: 312)
    }

    private func fixtures() throws -> [(name: String, presentation: NotchPresentation)] {
        let fixtures: [(String, [NotchWork], [NotchApproval], NotchModelPhase)] = [
            ("idle", [], [], .unloaded),
            ("queued", [work("queued", state: .queued)], [], .ready),
            ("loading", [work("loading", state: .waitingForModel)], [], .loading),
            ("thinking", [work("thinking", state: .running, origin: .conversation)], [], .ready),
            ("tool", [work("tool", state: .running, activity: .web)], [], .ready),
            ("approval", [work("approval", state: .waitingForApproval)], [
                .init(identity: "approval-a", workIdentity: "approval"),
            ], .ready),
            ("streaming", [work("streaming", state: .running, origin: .conversation, activity: .chat)], [], .ready),
            ("completion", [work("completion", state: .completed, attention: .unread)], [], .ready),
            ("failure", [work("failure", state: .failed, attention: .failed)], [], .ready),
            ("cancellation", [work("cancellation", state: .cancelled)], [], .unloaded),
            ("interruption", [work("interruption", state: .abandoned)], [], .unloaded),
        ]

        return try fixtures.enumerated().map { index, fixture in
            let snapshot = NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: UInt64(index + 1),
                daemonGeneration: "state-catalog",
                works: fixture.1,
                pendingApprovals: fixture.2,
                model: fixture.3
            )
            return (
                fixture.0,
                try NotchProjection.reduce(
                    previous: .idle,
                    input: .snapshot(snapshot),
                    reduceMotion: true
                )
            )
        }
    }

    private func work(
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
