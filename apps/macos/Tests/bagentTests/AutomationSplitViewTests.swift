import XCTest
@testable import bagent

final class AutomationSplitViewTests: XCTestCase {
    func testMasterRowsUseActiveFIFOThenNewestUnreadHistoryAndFourRowPaging() {
        let active = [
            AutomationMasterRow.active(
                runIdentity: "run-b",
                workIdentity: "work-b",
                displayName: "B",
                claimedOrder: 2),
            AutomationMasterRow.active(
                runIdentity: "run-a",
                workIdentity: "work-a",
                displayName: "A",
                claimedOrder: 1),
        ]
        let terminal = [
            AutomationMasterRow.terminal(
                sessionIdentity: "automation-session-old",
                runIdentity: "run-old",
                displayName: "Old",
                outcome: .completed,
                attention: .unread,
                finishedAt: Date(timeIntervalSince1970: 10)),
            AutomationMasterRow.terminal(
                sessionIdentity: "automation-session-new",
                runIdentity: "run-new",
                displayName: "New",
                outcome: .failed,
                attention: .unread,
                finishedAt: Date(timeIntervalSince1970: 20)),
        ]

        var navigator = AutomationSplitViewNavigator(
            projection: .make(active: active, unreadTerminal: terminal))

        XCTAssertEqual(navigator.rows.map(\.id), [
            "run-a", "run-b", "automation-session-new", "automation-session-old", "history",
        ])
        XCTAssertEqual(navigator.visibleRows.map(\.id), [
            "run-a", "run-b", "automation-session-new", "automation-session-old",
        ])

        XCTAssertTrue(navigator.moveSelection(by: 1))
        XCTAssertTrue(navigator.moveSelection(by: 1))
        XCTAssertTrue(navigator.moveSelection(by: 1))
        XCTAssertTrue(navigator.moveSelection(by: 1))
        XCTAssertEqual(navigator.selectedRow?.id, "history")
        XCTAssertEqual(navigator.visibleRows.map(\.id), [
            "run-b", "automation-session-new", "automation-session-old", "history",
        ])
        XCTAssertFalse(navigator.moveSelection(by: 1))
    }

    func testTerminalStatesExposeTextGlyphAndAccessibilityWithoutColor() {
        let rows = AutomationRunOutcome.allCases.map { outcome in
            AutomationMasterRow.terminal(
                sessionIdentity: "automation-session-\(outcome.rawValue)",
                runIdentity: "run-\(outcome.rawValue)",
                displayName: "Automation",
                outcome: outcome,
                attention: .unread,
                finishedAt: nil)
        }

        XCTAssertEqual(Set(rows.map(\.stateText)).count, rows.count)
        XCTAssertEqual(Set(rows.map(\.stateGlyph)).count, rows.count)
        XCTAssertEqual(Set(rows.map(\.accessibilityValue)).count, rows.count)
    }

    func testPreviewDoesNotAcknowledgeAndOpeningIsTheOnlyAcknowledgementTransition() {
        var navigator = AutomationSplitViewNavigator(projection: .make(
            active: [],
            unreadTerminal: [AutomationMasterRow.terminal(
                sessionIdentity: "automation-session-a",
                runIdentity: "run-a",
                displayName: "A",
                outcome: .partial,
                attention: .unread,
                finishedAt: Date())]))

        XCTAssertEqual(navigator.selectedRow?.attention, .unread)
        XCTAssertEqual(navigator.previewSelectedRow()?.attention, .unread)
        XCTAssertEqual(navigator.selectedRow?.attention, .unread)
        XCTAssertTrue(navigator.openSelectedTerminal())
        XCTAssertEqual(navigator.selectedRow?.attention, .viewed)
        XCTAssertEqual(navigator.depth, .detail)
    }
}
