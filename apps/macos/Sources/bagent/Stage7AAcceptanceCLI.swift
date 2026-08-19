import CryptoKit
import Foundation
import AppKit
import SwiftUI

enum Stage7AAcceptanceCLI {
    static let environmentKey = "BAGENT_STAGE7A_ACCEPTANCE_FIXTURE"

    static func run(outputURL: URL, sentinelURL: URL?) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            let (snapshot, restoredSelection, projection) = try await restoreInHostedUI()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
            let canonicalSnapshot = try encoder.encode(snapshot)
            let digest = SHA256.hash(data: canonicalSnapshot)
                .map { String(format: "%02x", $0) }
                .joined()
            let evidence: [String: Any] = [
                "ui_pid": ProcessInfo.processInfo.processIdentifier,
                "current_chat_identity": snapshot.identity,
                "current_chat_revision": snapshot.revision,
                "current_chat_content_sha256": digest,
                "turn_count": snapshot.turnCount,
                "draft_bytes": snapshot.draft?.text.utf8.count ?? 0,
                "draft_caret_utf16": restoredSelection.location,
                "draft_selection_length": restoredSelection.length,
                "submitted_attachment_count": projection.submittedAttachments,
                "unavailable_attachment_count": projection.unavailableAttachments,
                "validated_source_count": projection.validatedSources,
                "connector_reference_count": projection.connectorReferences,
                "approval_presentation_count": projection.approvalPresentations,
            ]
            let data = try JSONSerialization.data(
                withJSONObject: evidence,
                options: [.prettyPrinted, .sortedKeys])
            try data.write(to: outputURL, options: .atomic)
            if let sentinelURL {
                while FileManager.default.fileExists(atPath: sentinelURL.path) {
                    try await Task.sleep(for: .milliseconds(25))
                }
            }
            return 0
        } catch {
            return 1
        }
    }

    private struct ProjectionEvidence: Sendable {
        let submittedAttachments: Int
        let unavailableAttachments: Int
        let validatedSources: Int
        let connectorReferences: Int
        let approvalPresentations: Int
    }

    @MainActor
    private static func restoreInHostedUI() async throws -> (
        DaemonClient.CurrentChatSnapshot,
        NSRange,
        ProjectionEvidence
    ) {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 360),
            styleMask: [.titled],
            backing: .buffered,
            defer: false)
        window.contentView = NSHostingView(rootView: InlineNotchContent(viewModel: viewModel))
        window.makeKeyAndOrderFront(nil)
        var editor: NSTextView?
        for _ in 0..<80 {
            try await Task.sleep(for: .milliseconds(25))
            if let candidate = window.firstResponder as? NSTextView,
               candidate.isEditable {
                editor = candidate
                break
            }
        }
        guard editor != nil else {
            window.orderOut(nil)
            throw NSError(domain: "Stage7AAcceptanceCLI", code: 2)
        }
        await viewModel.restoreCurrentChat()

        for _ in 0..<80 {
            try await Task.sleep(for: .milliseconds(25))
            if let candidate = window.firstResponder as? NSTextView,
               candidate.isEditable,
               candidate.string == viewModel.inputText {
                editor = candidate
                let selection = candidate.selectedRange()
                if selection.location == candidate.string.utf16.count,
                   selection.length == 0 {
                    break
                }
            }
        }
        guard let editor else {
            window.orderOut(nil)
            throw NSError(domain: "Stage7AAcceptanceCLI", code: 3)
        }
        let selection = editor.selectedRange()
        guard editor.string == viewModel.inputText else {
            window.orderOut(nil)
            throw NSError(domain: "Stage7AAcceptanceCLI", code: 4)
        }
        guard let snapshot = viewModel.currentChatSnapshot else {
            window.orderOut(nil)
            throw NSError(domain: "Stage7AAcceptanceCLI", code: 5)
        }
        let projection = ProjectionEvidence(
            submittedAttachments: viewModel.restoredSubmittedAttachments.count,
            unavailableAttachments: viewModel.restoredSubmittedAttachments.filter {
                $0.availability == .unavailable
            }.count,
            validatedSources: viewModel.restoredValidatedSources.count,
            connectorReferences: viewModel.restoredConnectorReferences.count,
            approvalPresentations: viewModel.restoredApprovalPresentations.count)
        window.orderOut(nil)
        return (snapshot, selection, projection)
    }
}
