import XCTest
@testable import bagent

final class AutomationModelTests: XCTestCase {
    func testScheduleWireRoundTrip() throws {
        let daily = AutomationSchedule.recurring(
            rule: RecurrenceRuleWire(type: "daily", hours: nil, time: "08:00:00", day: nil, days: nil))
        let data = try JSONEncoder().encode(daily)
        let json = try XCTUnwrap(String(data: data, encoding: .utf8))
        XCTAssertTrue(json.contains("\"kind\":\"recurring\""))
        XCTAssertEqual(try JSONDecoder().decode(AutomationSchedule.self, from: data), daily)

        let once = AutomationSchedule.once(at: "2026-07-18T06:00:00Z")
        let onceData = try JSONEncoder().encode(once)
        XCTAssertEqual(try JSONDecoder().decode(AutomationSchedule.self, from: onceData), once)
    }

    func testRecordDecodingFromDaemonJSON() throws {
        let json = """
        {"id":"11111111-2222-3333-4444-555555555555","name":"Ranná pošta",
         "prompt":"skontroluj maily","enabled":true,"timezone":"Europe/Bratislava",
         "schedule":{"kind":"recurring","rule":{"type":"weekdays","time":"08:00:00"}},
         "next_run_at":"2026-07-20T06:00:00+00:00","created_at":"2026-07-17T10:00:00Z",
         "updated_at":"2026-07-17T10:00:00Z","last_run_at":null,
         "last_run_status":"partial","last_result_summary":"2 správy"}
        """
        let record = try JSONDecoder().decode(AutomationRecord.self, from: Data(json.utf8))
        XCTAssertEqual(record.name, "Ranná pošta")
        XCTAssertEqual(record.scheduleLabel, "po–pia o 08:00")
        XCTAssertEqual(record.lastRunStatus, "partial")
        XCTAssertNotNil(record.nextRunLabel)
    }

    func testRecurrenceDisplayLabels() {
        XCTAssertEqual(
            RecurrenceRuleWire(type: "every_n_hours", hours: 2, time: nil, day: nil, days: nil).displayLabel,
            "každé 2 h")
        XCTAssertEqual(
            RecurrenceRuleWire(type: "weekly", hours: nil, time: "07:45:00", day: "fri", days: nil).displayLabel,
            "týždenne pia o 07:45")
        XCTAssertEqual(
            RecurrenceRuleWire(type: "selected_weekdays", hours: nil, time: "09:00:00", day: nil, days: ["tue", "sat"]).displayLabel,
            "ut,so o 09:00")
    }
}

@MainActor
final class AutomationsSurfaceStateTests: XCTestCase {
    private func record(_ id: String, name: String) -> AutomationRecord {
        try! JSONDecoder().decode(AutomationRecord.self, from: Data("""
        {"id":"\(id)","name":"\(name)","prompt":"p","enabled":true,
         "timezone":"Europe/Bratislava","schedule":{"kind":"once","at":"2026-07-18T06:00:00Z"},
         "next_run_at":"2026-07-18T06:00:00Z","created_at":"2026-07-17T10:00:00Z",
         "updated_at":"2026-07-17T10:00:00Z","last_run_at":null,
         "last_run_status":null,"last_result_summary":null}
        """.utf8))
    }

    func testSelectionNavigationAndDetail() {
        let vm = ChatViewModel()
        vm.notchInteractionMode = .automations
        vm.automations = [record("a", name: "A"), record("b", name: "B"), record("c", name: "C")]
        XCTAssertTrue(vm.moveAutomationsSelection(by: 1))
        XCTAssertEqual(vm.automationsSelectionIndex, 1)
        XCTAssertTrue(vm.moveAutomationsSelection(by: -2)) // wraps
        XCTAssertEqual(vm.automationsSelectionIndex, 2)
        XCTAssertTrue(vm.openSelectedAutomationDetail())
        XCTAssertEqual(vm.automationsSurface, .detail("c"))
        // Arrows are not consumed on the detail page.
        XCTAssertFalse(vm.moveAutomationsSelection(by: 1))
    }

    func testEscapeBackNavigation() {
        let vm = ChatViewModel()
        vm.notchInteractionMode = .automations
        vm.automationsSurface = .deleteConfirmation("a")
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .detail("a"))
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .list)
        // At the list, Escape is not consumed → notch collapses.
        XCTAssertFalse(vm.automationsGoBack())
    }

    func testSlashCommandOpensAutomations() {
        let vm = ChatViewModel()
        vm.inputText = "/automations"
        XCTAssertTrue(vm.acceptSlashSuggestion())
        XCTAssertEqual(vm.notchInteractionMode, .automations)
        XCTAssertEqual(vm.automationsSurface, .list)
    }

    func testGeometryStaysWithinCeilings() {
        XCTAssertLessThanOrEqual(NotchWrapMetrics.automationsWingWidth, NotchWrapMetrics.maxWingWidth)
        XCTAssertLessThanOrEqual(NotchWrapMetrics.automationsBridgeHeight, NotchWrapMetrics.maxBridgeHeight)
    }
}
