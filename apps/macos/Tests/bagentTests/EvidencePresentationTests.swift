import XCTest
@testable import bagent

final class EvidenceEventDecodingTests: XCTestCase {
    func testDecodesCapturedSignedAcceptanceEventsWhenProvided() throws {
        guard let path = ProcessInfo.processInfo.environment["BAGENT_STAGE8_ACCEPTANCE_SSE"],
              !path.isEmpty
        else {
            throw XCTSkip("signed acceptance SSE capture not provided")
        }
        let contents = try String(contentsOfFile: path, encoding: .utf8)
        let payloads = contents.split(separator: "\n").map(String.init)
        XCTAssertFalse(payloads.isEmpty)

        var outcomes = 0
        var done = 0
        for payload in payloads {
            let decoded = try JSONDecoder().decode(SSEEvent.self, from: Data(payload.utf8))
            if decoded.type == "done" { done += 1 }
            if case .evidenceOutcome = DaemonClient.evidenceChatEvent(from: decoded) {
                outcomes += 1
            }
        }
        XCTAssertGreaterThan(outcomes, 0)
        XCTAssertEqual(outcomes, done)
    }

    func testDecodesEveryEvidenceEventShape() throws {
        let fixtures = [
            """
            {"type":"evidence_phase","turn_id":"turn-1","phase":"reading","completed":2,"total":3,
             "duration_ms":0,"timed_out":false,"fallback":false,"repair":false}
            """,
            """
            {"type":"logical_activity_started","turn_id":"turn-1","activity_id":"evidence:a",
             "normalized_operation":"mail.read","argument_hash":"hash","execution_status":"in_progress",
             "contribution":"empty","evidence_count":0,"source_domains":[],"duration_ms":0,
             "attempt_count":0,"retries":0,"duplicates_suppressed":0}
            """,
            """
            {"type":"logical_activity_completed","turn_id":"turn-1","activity_id":"evidence:a",
             "normalized_operation":"web.fetch","argument_hash":"hash","execution_status":"succeeded",
             "contribution":"satisfied","evidence_count":1,"source_domains":["example.com"],
             "duration_ms":125,"attempt_count":2,"retries":1,"duplicates_suppressed":0}
            """,
            """
            {"type":"evidence_polish","turn_id":"turn-1","status":"rejected"}
            """,
            """
            {"type":"evidence_outcome","turn_id":"turn-1","state":"verified","kind":"web",
             "acquired":2,"requested":2,"source_count":2,"message":"Web verified · 2 sources"}
            """,
        ]

        let decoded = try fixtures.map {
            try JSONDecoder().decode(SSEEvent.self, from: Data($0.utf8))
        }
        XCTAssertEqual(decoded.map(\.type), [
            "evidence_phase",
            "logical_activity_started",
            "logical_activity_completed",
            "evidence_polish",
            "evidence_outcome",
        ])
        XCTAssertEqual(decoded[0].phase, "reading")
        XCTAssertEqual(decoded[1].activity_id, "evidence:a")
        XCTAssertEqual(decoded[2].source_domains, ["example.com"])
        XCTAssertEqual(decoded[3].status, "rejected")
        XCTAssertEqual(decoded[4].outcome_kind, "web")

        let chatEvents = decoded.compactMap(DaemonClient.evidenceChatEvent(from:))
        XCTAssertEqual(chatEvents.count, 5)
        guard case .evidencePhase(let phase) = chatEvents[0],
              case .logicalActivityStarted(let started) = chatEvents[1],
              case .logicalActivityCompleted(let completed) = chatEvents[2],
              case .evidencePolish(let polish) = chatEvents[3],
              case .evidenceOutcome(let outcome) = chatEvents[4]
        else {
            return XCTFail("typed evidence event mapping changed")
        }
        XCTAssertEqual(phase.phase, .reading)
        XCTAssertEqual(started.executionStatus, .inProgress)
        XCTAssertEqual(completed.contribution, .satisfied)
        XCTAssertEqual(polish.status, .rejected)
        XCTAssertEqual(outcome.state, .verified)
    }
}

final class EvidencePresentationTests: XCTestCase {
    private func outcome(
        state: DaemonClient.EvidenceOutcomeState,
        kind: DaemonClient.EvidenceOutcomeKind,
        acquired: Int = 0,
        requested: Int = 0,
        sources: Int = 0
    ) -> DaemonClient.EvidenceOutcomeEvent {
        let message: String
        switch (state, kind) {
        case (.verified, .mail):
            message = "Read \(acquired) of \(requested) emails"
        case (.partial, .mail):
            message = "Read \(acquired) of \(requested) emails · partial"
        case (.verified, .web):
            message = "Web verified · \(sources) sources"
        case (.conflict, .web):
            message = "Web verified · \(sources) sources · conflict"
        case (.denied, .mail):
            message = "Mail access denied"
        case (.verificationShortfall, .web):
            message = "Couldn't verify sources"
        case (.empty, .mail):
            message = "No usable Mail evidence"
        case (.unavailable, .mail):
            message = "Mail unavailable"
        default:
            message = state.rawValue
        }
        return .init(
            turnId: "turn-1",
            state: state,
            kind: kind,
            acquired: acquired,
            requested: requested,
            sourceCount: sources,
            message: message
        )
    }

    func testCollapsedOutcomeLabelsDescribeEvidenceCompletion() {
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(
                state: .verified, kind: .mail, acquired: 3, requested: 3
            )),
            "Read 3 of 3 emails"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(
                state: .partial, kind: .mail, acquired: 2, requested: 3
            )),
            "Read 2 of 3 emails · partial"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(
                state: .verified, kind: .web, acquired: 2, requested: 2, sources: 2
            )),
            "Web verified · 2 sources"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(
                state: .conflict, kind: .web, acquired: 2, requested: 2, sources: 2
            )),
            "Web verified · 2 sources · conflict"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(state: .denied, kind: .mail)),
            "Mail access denied"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(
                state: .verificationShortfall, kind: .web
            )),
            "Couldn't verify sources"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(state: .empty, kind: .mail)),
            "No usable Mail evidence"
        )
        XCTAssertEqual(
            EvidencePresentation.outcomeLabel(outcome(state: .unavailable, kind: .mail)),
            "Mail unavailable"
        )
    }

    func testEveryRequiredPhaseHasMeaningfulLabelAndProgress() {
        let phases: [DaemonClient.EvidencePhase] = [
            .findingMail, .reading, .searching, .verifying,
            .loadingSynthesisModel, .preparingAnswer, .repairing,
            .fallingBack, .validating, .deterministicRendering,
        ]
        let labels = phases.map {
            EvidencePresentation.phaseLabel(.init(
                turnId: "turn-1", phase: $0, completed: 2, total: 3
            ))
        }
        XCTAssertEqual(Set(labels).count, phases.count)
        XCTAssertEqual(labels[1], "Reading 2 of 3")
        XCTAssertFalse(labels.contains("Thinking"))
    }

    func testExpandedDetailSeparatesExecutionContributionAndRetryMetadata() {
        let event = DaemonClient.LogicalActivityEvent(
            turnId: "turn-1",
            activityId: "evidence:a",
            normalizedOperation: "web.fetch",
            executionStatus: .succeeded,
            contribution: .empty,
            evidenceCount: 0,
            sourceDomains: ["example.com"],
            durationMs: 100,
            attemptCount: 2,
            retries: 1,
            duplicatesSuppressed: 1,
            failureReason: "empty_extraction"
        )
        let detail = EvidencePresentation.activityDetail(EvidenceLogicalActivity(event: event))
        XCTAssertTrue(detail.contains("empty"))
        XCTAssertTrue(detail.contains("example.com"))
        XCTAssertTrue(detail.contains("1 retries"))
        XCTAssertTrue(detail.contains("1 duplicates suppressed"))
        XCTAssertTrue(detail.contains("empty_extraction"))
        XCTAssertFalse(detail.contains("1 evidence"))
    }

    func testAccessibilityLabelIncludesOutcomeAndControlState() {
        let value = outcome(state: .partial, kind: .mail, acquired: 2, requested: 3)
        XCTAssertEqual(
            EvidencePresentation.accessibilityLabel(outcome: value, expanded: false),
            "Expand evidence activity. Read 2 of 3 emails · partial"
        )
        XCTAssertEqual(
            EvidencePresentation.accessibilityLabel(outcome: value, expanded: true),
            "Collapse evidence activity. Read 2 of 3 emails · partial"
        )
    }
}
