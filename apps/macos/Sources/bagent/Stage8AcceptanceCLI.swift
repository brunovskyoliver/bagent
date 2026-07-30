import CryptoKit
import Foundation

enum Stage8AcceptanceCLI {
    static let environmentKey = "BAGENT_STAGE8_ACCEPTANCE_FIXTURES"

    static func run(
        acquisition: String,
        polish: String,
        prompt: String,
        outputURL: URL
    ) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else {
            return 64
        }

        let client = DaemonClient()
        let health = await client.healthStatus()
        guard health.daemonUp else { return 69 }

        var presentation = ChatMessage(role: .assistant, content: "")
        var outcomeCount = 0
        var doneCount = 0
        var tokenText = ""
        var polishStatuses: [String] = []
        var phases: [[String: Any]] = []
        var providers: [[String: String]] = []
        do {
            try await client.configureStage8Acceptance(acquisition: acquisition, polish: polish)
            for try await event in client.chatStream(
                text: prompt,
                sessionId: "stage8-signed-swift-client-\(UUID().uuidString)",
                model: health.model
            ) {
                _ = EvidencePresentation.apply(event, to: &presentation)
                switch event {
                case .token(let token):
                    tokenText += token
                case .evidencePhase(let phase):
                    phases.append([
                        "phase": phase.phase.rawValue,
                        "completed": phase.completed.map { $0 as Any } ?? NSNull(),
                        "total": phase.total.map { $0 as Any } ?? NSNull(),
                    ])
                case .evidenceOutcome:
                    outcomeCount += 1
                case .evidencePolish(let polish):
                    polishStatuses.append(polish.status.rawValue)
                case .evidenceAcquisitionDiagnostic(let diagnostic):
                    if diagnostic.status == "search_completed", let provider = diagnostic.provider,
                       let providerStatus = diagnostic.providerStatus {
                        providers.append(["provider": provider, "status": providerStatus])
                    }
                case .done:
                    doneCount += 1
                default:
                    break
                }
            }
        } catch {
            return 70
        }

        guard let outcome = presentation.evidenceOutcome else { return 65 }
        let activities: [[String: Any]] = presentation.evidenceActivities.map { activity in
            [
                "operation": activity.operation,
                "argument_hash": activity.argumentHash,
                "execution_status": activity.executionStatus.rawValue,
                "contribution": activity.contribution.rawValue,
                "evidence_count": activity.evidenceCount,
                "attempt_count": activity.attemptCount,
                "retries": activity.retries,
                "duplicates_suppressed": activity.duplicatesSuppressed,
                "failure_reason": activity.failureReason.map { $0 as Any } ?? NSNull(),
            ]
        }
        let urls = Self.urls(in: tokenText).sorted()
        let result: [String: Any] = [
            "activities": activities,
            "citation_set_sha256": Self.sha256(urls.joined(separator: "\n")),
            "done_count": doneCount,
            "outcome": [
                "state": outcome.state.rawValue,
                "kind": outcome.kind.rawValue,
                "acquired": outcome.acquired,
                "requested": outcome.requested,
                "source_count": outcome.sourceCount,
            ],
            "outcome_count": outcomeCount,
            "phases": phases,
            "polish_statuses": polishStatuses,
            "providers": providers,
            "token_bytes": tokenText.utf8.count,
            "token_sha256": Self.sha256(tokenText),
            "ui_activity_count": presentation.evidenceActivities.count,
            "ui_outcome_label_sha256": Self.sha256(EvidencePresentation.outcomeLabel(outcome)),
            "ui_outcome_present": true,
            "ui_polish_status": (presentation.evidencePolishStatus?.rawValue).map { $0 as Any } ?? NSNull(),
        ]
        guard outcomeCount == 1, doneCount == 1,
              let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]),
              (try? data.write(to: outputURL, options: .atomic)) != nil
        else { return 65 }
        return 0
    }

    private static func sha256(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private static func urls(in value: String) -> [String] {
        let pattern = #"https?://[^\s)\]]+"#
        guard let expression = try? NSRegularExpression(pattern: pattern) else { return [] }
        let range = NSRange(value.startIndex..., in: value)
        return expression.matches(in: value, range: range).compactMap { match in
            Range(match.range, in: value).map { String(value[$0]) }
        }
    }
}
