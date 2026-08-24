import XCTest
@testable import bagent

final class UIRelaunchHandoffTests: XCTestCase {
    func testHandoffRoundTripsOnlyTheBoundedAllowlist() throws {
        let handoff = try UIRelaunchHandoff(
            createdAt: Date(timeIntervalSince1970: 100),
            sourceUIIdentity: "old-ui",
            replacementUIIdentity: "new-ui",
            currentChatIdentity: "chat-1",
            refetchCursor: 42,
            draft: "draft",
            caretOffset: 5,
            selectionLength: 0,
            pendingAttachmentReferences: ["attachment-1"],
            selectedArea: .privacyAndPermissions,
            selectedChild: .fullDiskAccess,
            permissionPhase: .daemonPreservingRelaunchHandoff,
            semanticFocus: "permission-open"
        )
        let data = try UIRelaunchHandoffCodec.encode(handoff)
        let decoded = try UIRelaunchHandoffCodec.decode(data, now: Date(timeIntervalSince1970: 101))

        XCTAssertEqual(decoded, handoff)
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("transcript"))
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("token"))
    }

    func testHandoffRejectsExpiryIdentityVersionReplayAndOversizedDraft() throws {
        let store = InMemoryProtectedHandoffStore()
        let handoff = try UIRelaunchHandoff(
            createdAt: Date(timeIntervalSince1970: 100),
            sourceUIIdentity: "old-ui",
            replacementUIIdentity: "new-ui",
            currentChatIdentity: "chat-1",
            refetchCursor: nil,
            draft: "draft",
            caretOffset: 5,
            selectionLength: 0,
            pendingAttachmentReferences: [],
            selectedArea: .general,
            selectedChild: nil,
            permissionPhase: .grantedButUIRelaunchRequired,
            semanticFocus: "heading"
        )
        let token = try store.write(handoff)

        XCTAssertThrowsError(try store.consume(token: token, source: "wrong", replacement: "new-ui", now: Date(timeIntervalSince1970: 101)))
        XCTAssertThrowsError(try store.consume(token: token, source: "old-ui", replacement: "new-ui", now: Date(timeIntervalSince1970: 161)))

        let replayToken = try store.write(handoff)
        XCTAssertEqual(try store.consume(token: replayToken, source: "old-ui", replacement: "new-ui", now: Date(timeIntervalSince1970: 101)), handoff)
        XCTAssertThrowsError(try store.consume(token: replayToken, source: "old-ui", replacement: "new-ui", now: Date(timeIntervalSince1970: 101)))

        XCTAssertThrowsError(try UIRelaunchHandoff(
            createdAt: Date(timeIntervalSince1970: 100),
            sourceUIIdentity: "old-ui", replacementUIIdentity: "new-ui", currentChatIdentity: "chat-1",
            refetchCursor: nil, draft: String(repeating: "x", count: 16 * 1024 + 1), caretOffset: 0,
            selectionLength: 0, pendingAttachmentReferences: [], selectedArea: .general,
            selectedChild: nil, permissionPhase: .unknown, semanticFocus: "heading"
        ))
    }

    func testDecodedHandoffStillValidatesBoundsAndExpiry() throws {
        let handoff = try UIRelaunchHandoff(
            createdAt: Date(timeIntervalSince1970: 100),
            sourceUIIdentity: "old-ui",
            replacementUIIdentity: "new-ui",
            currentChatIdentity: "chat-1",
            refetchCursor: nil,
            draft: "draft",
            caretOffset: 1,
            selectionLength: 0,
            pendingAttachmentReferences: [],
            selectedArea: .general,
            selectedChild: nil,
            permissionPhase: .unknown,
            semanticFocus: "input"
        )
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: UIRelaunchHandoffCodec.encode(handoff)) as? [String: Any])
        object["selectionLength"] = 10_000
        let invalid = try JSONSerialization.data(withJSONObject: object)
        XCTAssertThrowsError(try UIRelaunchHandoffCodec.decode(invalid, now: Date(timeIntervalSince1970: 101)))
    }

    func testRelaunchEligibilityRejectsTurnsAndApprovalsButAllowsAutomation() {
        XCTAssertFalse(UIOnlyRelaunchEligibility.isAllowed(activeConversationTurn: true, pendingApproval: false))
        XCTAssertFalse(UIOnlyRelaunchEligibility.isAllowed(activeConversationTurn: false, pendingApproval: true))
        XCTAssertTrue(UIOnlyRelaunchEligibility.isAllowed(activeConversationTurn: false, pendingApproval: false))
        XCTAssertTrue(UIOnlyRelaunchEligibility.automationSurvives)
    }

    func testOwnershipPlanDoesNotIncludeDaemonLifecycle() {
        XCTAssertEqual(
            UIOnlyRelaunchOwnership.allowedActions,
            [.buildHandoff, .launchReplacement, .consumeHandoff, .refetch, .fenceUI, .activateReplacement, .probe]
        )
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.launchDaemon))
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.restartBaseRT))
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.mutateAutomationWork))
    }
}
