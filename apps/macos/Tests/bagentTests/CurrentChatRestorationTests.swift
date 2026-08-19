import XCTest
@testable import bagent

@MainActor
final class CurrentChatRestorationTests: XCTestCase {
    func testReopenProjectsAllRetainedRecordsAndUnavailableAttachment() {
        let snapshot = DaemonClient.CurrentChatSnapshot(
            identity: "chat-a",
            revision: 7,
            turnCount: 1,
            contentBytes: 512,
            turns: [DaemonClient.CurrentChatTurn(
                identity: "turn-a",
                userMessage: "prompt",
                assistantOutput: "answer",
                state: "completed",
                interruptionReason: nil,
                submittedAt: "2026-08-20T10:00:00Z",
                completedAt: "2026-08-20T10:00:01Z")],
            draft: nil,
            continuation: nil,
            submittedAttachments: [DaemonClient.SubmittedAttachment(
                conversationTurnIdentity: "turn-a",
                identity: "attachment-a",
                filename: "missing.txt",
                mime: "text/plain",
                sizeBytes: 7,
                available: false)],
            validatedSources: [DaemonClient.CurrentChatAvailability(
                identity: "source-a", label: "Source", availability: "available")],
            connectorReferences: [DaemonClient.CurrentChatAvailability(
                identity: "reference-a", label: "mail", availability: "unavailable")],
            completedApprovalPresentations: [DaemonClient.CompletedApprovalPresentation(
                identity: "approval-a", category: "filesystem_write", outcome: "allowed")])
        let viewModel = ChatViewModel(startMonitoring: false)

        viewModel.applyCurrentChatSnapshot(snapshot, rebuildMessages: true)

        XCTAssertEqual(viewModel.messages.first?.attachments.count, 1)
        XCTAssertEqual(viewModel.messages.first?.attachments.first?.availability, .unavailable)
        XCTAssertNil(viewModel.messages.first?.attachments.first?.localURL)
        XCTAssertEqual(viewModel.restoredSubmittedAttachments.first?.availability, .unavailable)
        XCTAssertEqual(viewModel.restoredValidatedSources, snapshot.validatedSources)
        XCTAssertEqual(viewModel.restoredConnectorReferences, snapshot.connectorReferences)
        XCTAssertEqual(viewModel.restoredApprovalPresentations, snapshot.completedApprovalPresentations)
    }

    func testMissingPendingReferenceRemainsVisibleAndRemovable() {
        let missing = ChatViewModel.unavailablePendingAttachment(identity: "missing-a")

        XCTAssertEqual(missing.id, "missing-a")
        XCTAssertEqual(missing.availability, .unavailable)
        XCTAssertNil(missing.localURL)
    }

    func testRejectedAdmissionsRestoreExactDraftAndPendingReferences() {
        let attachment = ChatAttachment(
            id: "attachment-a",
            filename: "draft.txt",
            mime: "text/plain",
            kind: .text,
            localURL: nil,
            sizeBytes: 12,
            availability: .available)
        let exactDraft = "  /settings  \nunchanged"

        for rejection in ["stale revision", "content bound", "missing attachment"] {
            let viewModel = ChatViewModel(startMonitoring: false)
            viewModel.applyRejectedSubmissionDraft(
                text: exactDraft,
                attachments: [attachment],
                availableAttachmentIdentities: rejection == "missing attachment" ? [] : [attachment.id])

            XCTAssertEqual(viewModel.inputText, exactDraft, rejection)
            XCTAssertEqual(viewModel.pendingAttachments.map(\.id), [attachment.id], rejection)
            XCTAssertEqual(viewModel.restoredPendingAttachmentReferences, [attachment.id], rejection)
            XCTAssertEqual(
                viewModel.pendingAttachments.first?.availability,
                rejection == "missing attachment" ? .unavailable : .available,
                rejection)
        }
    }

    func testRefetchedNewTurnProvesAdmissionBeforeAnyStreamEvent() {
        let before = snapshot(identity: "chat-a", turns: [])
        let admitted = snapshot(identity: "chat-a", turns: [DaemonClient.CurrentChatTurn(
            identity: "turn-new",
            userMessage: "exact prompt",
            assistantOutput: nil,
            state: "active",
            interruptionReason: nil,
            submittedAt: "2026-08-20T10:00:00Z",
            completedAt: nil)])

        XCTAssertTrue(ChatViewModel.submissionWasAdmitted(
            before: before,
            after: admitted,
            exactText: "exact prompt"))
        XCTAssertFalse(ChatViewModel.submissionWasAdmitted(
            before: before,
            after: admitted,
            exactText: "different prompt"))
    }

    private func snapshot(
        identity: String,
        turns: [DaemonClient.CurrentChatTurn]
    ) -> DaemonClient.CurrentChatSnapshot {
        DaemonClient.CurrentChatSnapshot(
            identity: identity,
            revision: 1,
            turnCount: UInt64(turns.count),
            contentBytes: 0,
            turns: turns,
            draft: nil,
            continuation: nil,
            submittedAttachments: [],
            validatedSources: [],
            connectorReferences: [],
            completedApprovalPresentations: [])
    }
}
