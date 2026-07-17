import XCTest
@testable import bagent

final class AutomationDraftBuilderTests: XCTestCase {
    private var cal: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "Europe/Bratislava")!
        return c
    }

    private func date(_ y: Int, _ mo: Int, _ d: Int, _ h: Int, _ mi: Int) -> Date {
        cal.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
    }

    func testScheduledDateForTodayTomorrowAndCustom() {
        let now = date(2026, 7, 17, 10, 0)
        var draft = AutomationDraft(now: now, calendar: cal)
        draft.hour = 14
        draft.minute = 30

        draft.day = .today
        XCTAssertEqual(AutomationDraftBuilder.scheduledDate(draft, calendar: cal, now: now),
                       date(2026, 7, 17, 14, 30))
        draft.day = .tomorrow
        XCTAssertEqual(AutomationDraftBuilder.scheduledDate(draft, calendar: cal, now: now),
                       date(2026, 7, 18, 14, 30))
        draft.day = .custom(date(2026, 8, 1, 0, 0))
        XCTAssertEqual(AutomationDraftBuilder.scheduledDate(draft, calendar: cal, now: now),
                       date(2026, 8, 1, 14, 30))
    }

    func testWireDraftEncodesOnceScheduleWithZone() throws {
        let now = date(2026, 7, 17, 10, 0)
        var draft = AutomationDraft(now: now, calendar: cal)
        draft.name = "  Faktúry  "
        draft.prompt = "nájdi faktúry so splatnosťou"
        draft.day = .tomorrow
        draft.hour = 9
        draft.minute = 30
        let wire = try XCTUnwrap(AutomationDraftBuilder.wireDraft(
            draft, timezone: "Europe/Bratislava", calendar: cal, now: now))
        XCTAssertEqual(wire.name, "Faktúry")
        XCTAssertEqual(wire.prompt, "nájdi faktúry so splatnosťou")
        XCTAssertEqual(wire.timezone, "Europe/Bratislava")
        // 09:30 Bratislava summer = 07:30 UTC.
        XCTAssertEqual(wire.schedule, .once(at: "2026-07-18T07:30:00Z"))
        // Full body encodes for POST /automations.
        let body = try JSONEncoder().encode(wire)
        let json = String(decoding: body, as: UTF8.self)
        XCTAssertTrue(json.contains("\"kind\":\"once\""))
        XCTAssertTrue(json.contains("Faktúry")) // diacritics preserved
    }
}

@MainActor
final class AutomationEditorFlowTests: XCTestCase {
    func testEditorStepNavigationAndValidation() {
        let vm = ChatViewModel()
        vm.notchInteractionMode = .automations
        vm.startAutomationCreation()
        XCTAssertEqual(vm.automationsSurface, .editorTask)

        // Empty task blocks advancing with a visible reason.
        vm.automationEditorNext()
        XCTAssertEqual(vm.automationsSurface, .editorTask)
        XCTAssertNotNil(vm.automationsError)

        vm.automationDraft.name = "Ranná pošta"
        vm.automationDraft.prompt = "zhrň urgentné maily"
        vm.automationEditorNext()
        XCTAssertEqual(vm.automationsSurface, .editorSchedule)
        XCTAssertNil(vm.automationsError)

        // A schedule in the past is rejected before review.
        vm.automationDraft.day = .today
        vm.automationDraft.hour = 0
        vm.automationDraft.minute = 0
        vm.automationEditorNext()
        XCTAssertEqual(vm.automationsSurface, .editorSchedule)
        XCTAssertEqual(vm.automationsError, "Čas už uplynul")

        vm.automationDraft.day = .tomorrow
        vm.automationEditorNext()
        XCTAssertEqual(vm.automationsSurface, .editorReview)
        XCTAssertTrue(vm.automationDraftSummary.contains("Ranná pošta"))

        // Escape steps backwards through the flow, then discards to the list.
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .editorSchedule)
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .editorTask)
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .list)
    }

    func testEditPrefillsDraftFromRecord() throws {
        let vm = ChatViewModel()
        let record = try JSONDecoder().decode(AutomationRecord.self, from: Data("""
        {"id":"abc","name":"Faktúry","prompt":"nájdi faktúry","enabled":false,
         "timezone":"Europe/Bratislava","schedule":{"kind":"once","at":"2026-07-18T07:30:00Z"},
         "next_run_at":"2026-07-18T07:30:00Z","created_at":"2026-07-17T10:00:00Z",
         "updated_at":"2026-07-17T10:00:00Z","last_run_at":null,
         "last_run_status":null,"last_result_summary":null}
        """.utf8))
        vm.startAutomationEdit(record)
        XCTAssertEqual(vm.automationsSurface, .editorTask)
        XCTAssertEqual(vm.automationDraft.editingID, "abc")
        XCTAssertEqual(vm.automationDraft.name, "Faktúry")
        XCTAssertEqual(vm.automationDraft.prompt, "nájdi faktúry")
        XCTAssertFalse(vm.automationDraft.enabled)
        // Escape from an edit returns to the record's detail, keeping context.
        XCTAssertTrue(vm.automationsGoBack())
        XCTAssertEqual(vm.automationsSurface, .detail("abc"))
    }
}
