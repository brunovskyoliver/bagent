import Foundation

enum Stage8AcceptanceCLI {
    static let environmentKey = "BAGENT_STAGE8_ACCEPTANCE_FIXTURES"

    static func run(prompt: String, outputURL: URL) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else {
            return 64
        }

        let client = DaemonClient()
        let health = await client.healthStatus()
        guard health.daemonUp else { return 69 }

        var outcomeCount = 0
        var doneCount = 0
        var tokenBytes = 0
        var polishStatuses: [String] = []
        do {
            for try await event in client.chatStream(
                text: prompt,
                sessionId: "stage8-signed-swift-client",
                model: health.model
            ) {
                switch event {
                case .token(let token):
                    tokenBytes += token.utf8.count
                case .evidenceOutcome:
                    outcomeCount += 1
                case .evidencePolish(let polish):
                    polishStatuses.append(polish.status.rawValue)
                case .done:
                    doneCount += 1
                default:
                    break
                }
            }
        } catch {
            return 70
        }

        let result: [String: Any] = [
            "done_count": doneCount,
            "outcome_count": outcomeCount,
            "polish_statuses": polishStatuses,
            "token_bytes": tokenBytes,
        ]
        guard outcomeCount == 1, doneCount == 1,
              let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]),
              (try? data.write(to: outputURL, options: .atomic)) != nil
        else { return 65 }
        return 0
    }
}
