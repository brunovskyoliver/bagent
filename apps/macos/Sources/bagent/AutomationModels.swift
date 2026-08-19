import Foundation

/// Wire model of the daemon's typed schedule:
/// `{"kind":"once","at":…}` or `{"kind":"recurring","rule":{…}}`.
enum AutomationSchedule: Equatable, Codable, Sendable {
    case once(at: String)
    case recurring(rule: RecurrenceRuleWire)

    private enum CodingKeys: String, CodingKey { case kind, at, rule }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "once":
            self = .once(at: try c.decode(String.self, forKey: .at))
        case "recurring":
            self = .recurring(rule: try c.decode(RecurrenceRuleWire.self, forKey: .rule))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: c, debugDescription: "unknown schedule kind \(other)")
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .once(let at):
            try c.encode("once", forKey: .kind)
            try c.encode(at, forKey: .at)
        case .recurring(let rule):
            try c.encode("recurring", forKey: .kind)
            try c.encode(rule, forKey: .rule)
        }
    }
}

/// Matches the Rust `RecurrenceRule` tagged enum. `type` is one of
/// every_n_hours / daily / weekdays / selected_weekdays / weekly; the other
/// fields apply per type. Weekdays are lowercase three-letter (mon…sun);
/// times are local "HH:mm:ss".
struct RecurrenceRuleWire: Codable, Equatable, Sendable {
    var type: String
    var hours: Int?
    var time: String?
    var day: String?
    var days: [String]?

    /// Compact Slovak label for the notch ("denne o 08:00", "po–pia o 08:00").
    var displayLabel: String {
        let t = time.map { String($0.prefix(5)) } ?? ""
        switch type {
        case "every_n_hours":
            return "každé \(hours ?? 0) h"
        case "daily":
            return "denne o \(t)"
        case "weekdays":
            return "po–pia o \(t)"
        case "selected_weekdays":
            let names = (days ?? []).map { Self.shortDay($0) }.joined(separator: ",")
            return "\(names) o \(t)"
        case "weekly":
            return "týždenne \(Self.shortDay(day ?? "")) o \(t)"
        default:
            return type
        }
    }

    static func shortDay(_ wire: String) -> String {
        switch wire {
        case "mon": return "po"
        case "tue": return "ut"
        case "wed": return "st"
        case "thu": return "št"
        case "fri": return "pia"
        case "sat": return "so"
        case "sun": return "ne"
        default: return wire
        }
    }
}

struct AutomationRecord: Decodable, Identifiable, Equatable, Sendable {
    let id: String
    let definitionRevision: Int?
    let name: String
    let prompt: String
    let enabled: Bool
    let timezone: String
    let schedule: AutomationSchedule
    let nextRunAt: String?
    let lastRunAt: String?
    let lastRunStatus: String?
    let lastResultSummary: String?

    enum CodingKeys: String, CodingKey {
        case id, name, prompt, enabled, timezone, schedule
        case definitionRevision = "definition_revision"
        case nextRunAt = "next_run_at"
        case lastRunAt = "last_run_at"
        case lastRunStatus = "last_run_status"
        case lastResultSummary = "last_result_summary"
    }

    /// Compact schedule label ("zajtra 09:30", "denne o 08:00").
    var scheduleLabel: String {
        switch schedule {
        case .once:
            return AutomationTimeFormat.shortLocal(nextRunAt) ?? "—"
        case .recurring(let rule):
            return rule.displayLabel
        }
    }

    var nextRunLabel: String? {
        AutomationTimeFormat.shortLocal(nextRunAt)
    }
}

struct AutomationRunRecord: Decodable, Identifiable, Equatable, Sendable {
    let id: String
    let status: String
    let scheduledFor: String
    let finishedAt: String?
    let resultSummary: String?
    let isCatchUp: Bool
    let isManual: Bool

    enum CodingKeys: String, CodingKey {
        case id, status
        case scheduledFor = "scheduled_for"
        case finishedAt = "finished_at"
        case resultSummary = "result_summary"
        case isCatchUp = "is_catch_up"
        case isManual = "is_manual"
    }
}

struct AutomationSessionRecord: Decodable, Sendable, Equatable {
    struct TaskSnapshot: Decodable, Sendable, Equatable {
        let automationIdentity: String
        let automationRunIdentity: String
        let automationSessionIdentity: String
        let displayName: String
        let taskText: String
        let scheduleJSON: String
        let timezone: String
        let definitionRevision: Int64

        enum CodingKeys: String, CodingKey {
            case automationIdentity = "automation_identity"
            case automationRunIdentity = "automation_run_identity"
            case automationSessionIdentity = "automation_session_identity"
            case displayName = "display_name"
            case taskText = "task_text"
            case scheduleJSON = "schedule_json"
            case timezone
            case definitionRevision = "definition_revision"
        }
    }

    struct Activity: Decodable, Sendable, Equatable {
        let category: String
        let caption: String
        let safetyRelevant: Bool

        enum CodingKeys: String, CodingKey {
            case category, caption
            case safetyRelevant = "safety_relevant"
        }
    }

    struct ValidatedSource: Decodable, Sendable, Equatable {
        let sourceIdentity: String
        let label: String

        enum CodingKeys: String, CodingKey {
            case sourceIdentity = "source_identity"
            case label
        }
    }

    struct ConnectorReference: Decodable, Sendable, Equatable {
        let connectorKind: String
        let availability: String

        enum CodingKeys: String, CodingKey {
            case connectorKind = "connector_kind"
            case availability
        }
    }

    struct HistoricalApproval: Decodable, Sendable, Equatable {
        let category: String
        let sideEffectClass: String
        let occurredAt: String
        let resolution: String
        let origin: String
        let sessionScopedFingerprint: String

        enum CodingKeys: String, CodingKey {
            case category
            case sideEffectClass = "side_effect_class"
            case occurredAt = "occurred_at"
            case resolution, origin
            case sessionScopedFingerprint = "session_scoped_fingerprint"
        }
    }

    struct TruncationDisclosure: Decodable, Sendable, Equatable {
        let section: String
        let originalExtent: Int
        let retainedExtent: Int
        let reason: String

        enum CodingKeys: String, CodingKey {
            case section
            case originalExtent = "original_extent"
            case retainedExtent = "retained_extent"
            case reason
        }
    }

    let taskSnapshot: TaskSnapshot
    let outcome: String
    let finishedAt: String
    let resultSummary: String?
    let finalOutput: String?
    let finalOutputAvailable: Bool
    let activityTimeline: [Activity]
    let validatedSources: [ValidatedSource]
    let connectorReferences: [ConnectorReference]
    let historicalApprovals: [HistoricalApproval]
    let truncationDisclosures: [TruncationDisclosure]
    let attention: String

    enum CodingKeys: String, CodingKey {
        case taskSnapshot = "task_snapshot"
        case outcome
        case finishedAt = "finished_at"
        case resultSummary = "result_summary"
        case finalOutput = "final_output"
        case finalOutputAvailable = "final_output_available"
        case activityTimeline = "activity_timeline"
        case validatedSources = "validated_sources"
        case connectorReferences = "connector_references"
        case historicalApprovals = "historical_approvals"
        case truncationDisclosures = "truncation_disclosures"
        case attention
    }
}

struct AutomationContinuationProvenance: Decodable, Sendable, Equatable {
    let identity: String
    let sourceAutomationSessionIdentity: String
    let targetCurrentChatIdentity: String
    let seed: String
    let sourceDeleted: Bool

    enum CodingKeys: String, CodingKey {
        case identity, seed
        case sourceAutomationSessionIdentity = "source_automation_session_identity"
        case targetCurrentChatIdentity = "target_current_chat_identity"
        case sourceDeleted = "source_deleted"
    }
}

enum AutomationTimeFormat {
    static func parse(_ s: String?) -> Date? {
        guard let s else { return nil }
        // Formatters are not Sendable — build per call (parsing is rare).
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = iso.date(from: s) { return d }
        return ISO8601DateFormatter().date(from: s)
    }

    /// "dnes 14:00" / "zajtra 09:30" / "18.7. 06:00" in the local zone.
    static func shortLocal(_ isoString: String?, now: Date = Date()) -> String? {
        guard let date = parse(isoString) else { return nil }
        let cal = Calendar.current
        let time = date.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute())
        if cal.isDate(date, inSameDayAs: now) { return "dnes \(time)" }
        if let tomorrow = cal.date(byAdding: .day, value: 1, to: now),
           cal.isDate(date, inSameDayAs: tomorrow) {
            return "zajtra \(time)"
        }
        let day = cal.component(.day, from: date)
        let month = cal.component(.month, from: date)
        return "\(day).\(month). \(time)"
    }

    /// Compact status glyph for list rows.
    static func statusGlyph(_ status: String?) -> String? {
        switch status {
        case "completed": return "checkmark.circle"
        case "partial": return "exclamationmark.circle"
        case "failed", "abandoned": return "xmark.circle"
        case "running": return "circle.dotted"
        case "skipped_overlap", "skipped_stale": return "forward.circle"
        default: return nil
        }
    }
}
