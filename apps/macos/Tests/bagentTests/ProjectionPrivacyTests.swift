import XCTest
@testable import bagent

final class ProjectionPrivacyTests: XCTestCase {
    func testSnapshotDecoderRejectsUnknownRawToolArguments() {
        let json = #"""
        {
          "schemaVersion": 1,
          "cursor": 1,
          "daemonGeneration": "daemon-a",
          "works": [{
            "identity": "work-a",
            "revision": 1,
            "origin": "automation",
            "state": "running",
            "activity": {"category": "generic_tool"},
            "queuePosition": null,
            "automationDisplayName": "Morning check",
            "terminalAttention": null,
            "claimedOrder": 1,
            "raw_tool_arguments": "CANARY-SECRET"
          }],
          "pendingApprovals": [],
          "model": "ready"
        }
        """#.data(using: .utf8)!

        XCTAssertThrowsError(try NotchProjectionDecoder.decodeSnapshot(json))
    }

    func testEveryForbiddenCompactFieldFailsClosedWithoutEchoingItsValue() throws {
        let forbiddenFields = [
            "hiddenReasoning", "prompt", "automationTaskText", "rawToolName", "toolArguments",
            "connectorIdentifier", "evidenceContent", "sourcePassage", "providerError", "credential",
            "toolOutput", "modelOutput", "privateIdentity",
        ]

        for field in forbiddenFields {
            let object: [String: Any] = [
                "schemaVersion": 1,
                "cursor": 1,
                "daemonGeneration": "daemon-a",
                "works": [[
                    "identity": "opaque-work-a",
                    "revision": 1,
                    "origin": "automation",
                    "state": "running",
                    "activity": ["category": "generic_tool"],
                    "queuePosition": NSNull(),
                    "automationDisplayName": NSNull(),
                    "terminalAttention": NSNull(),
                    "claimedOrder": 1,
                    field: "FORBIDDEN-CANARY",
                ]],
                "pendingApprovals": [],
                "model": "ready",
            ]
            let data = try JSONSerialization.data(withJSONObject: object)

            XCTAssertThrowsError(try NotchProjectionDecoder.decodeSnapshot(data)) { error in
                XCTAssertFalse(String(reflecting: error).contains("FORBIDDEN-CANARY"))
            }
        }
    }

    func testUnknownActivityCategoryMapsToGenericWithoutRenderingRawName() throws {
        let json = #"""
        {
          "schemaVersion": 1,
          "cursor": 1,
          "daemonGeneration": "daemon-a",
          "works": [{
            "identity": "work-a",
            "revision": 1,
            "origin": "automation",
            "state": "running",
            "activity": {"category": "private_connector_CANARY"},
            "queuePosition": null,
            "automationDisplayName": null,
            "terminalAttention": null,
            "claimedOrder": 1
          }],
          "pendingApprovals": [],
          "model": "ready"
        }
        """#.data(using: .utf8)!

        let snapshot = try NotchProjectionDecoder.decodeSnapshot(json)
        let presentation = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))

        XCTAssertEqual(presentation.rail.activityCategory, .genericTool)
        XCTAssertEqual(presentation.rail.caption, "Using a tool")
        XCTAssertFalse(presentation.rail.accessibilityValue.contains("CANARY"))
        XCTAssertFalse(presentation.rail.caption.contains("CANARY"))
        XCTAssertFalse(presentation.statusPill.accessibilityValue.contains("CANARY"))
    }
}
