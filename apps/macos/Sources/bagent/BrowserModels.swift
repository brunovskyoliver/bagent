import Foundation

enum BrowserErrorCode: String, Codable, Sendable {
    case navigationBlocked = "navigation_blocked"
    case mixedDNSAnswer = "mixed_dns_answer"
    case staleElementReference = "stale_element_reference"
    case visibleInteractionRequired = "visible_interaction_required"
    case controlRevokedByUser = "control_revoked_by_user"
    case submissionGrantRequired = "submission_grant_required"
    case passwordFieldForbidden = "password_field_forbidden"
    case permissionNotSupported = "permission_not_supported"
    case popupNotSupported = "popup_not_supported"
    case sessionLimitReached = "session_limit_reached"
    case browserAppUnavailable = "browser_app_unavailable"
    case browserDisabled = "browser_disabled"
    case browserProcessTerminated = "browser_process_terminated"
    case operationTimedOut = "operation_timed_out"
    case sessionNotFound = "session_not_found"
    case connectionNotOwner = "connection_not_owner"
    case invalidRequest = "invalid_request"
    case approvalRequired = "approval_required"
}

struct BrowserFailure: Codable, Equatable, Sendable, Error {
    let code: BrowserErrorCode
    let message: String
    let details: [String: String]

    init(_ code: BrowserErrorCode, _ message: String, details: [String: String] = [:]) {
        self.code = code
        self.message = message
        self.details = details
    }
}

enum BrowserRuntimeState: String, Codable, Sendable {
    case starting
    case ready
    case terminating
}

enum BrowserPageState: String, Codable, Sendable {
    case empty
    case loading
    case interactive
    case failed
}

enum BrowserVisibility: String, Codable, Sendable {
    case hidden
    case popup
}

enum BrowserOwnership: String, Codable, Sendable {
    case connected
    case detached
    case reclaimPending = "reclaim_pending"
}

enum BrowserControlState: String, Codable, Sendable {
    case agent
    case user
    case waitingForUser = "waiting_for_user"
}

enum BrowserCueState: String, Codable, Sendable {
    case steady
    case active
    case attention
    case detached
    case reclaimPending = "reclaim_pending"
}

struct BrowserRect: Codable, Equatable, Sendable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

struct BrowserViewport: Codable, Equatable, Sendable {
    let width: Int
    let height: Int
    let backingScale: Double
}

struct BrowserPageInfo: Codable, Equatable, Sendable {
    let url: String?
    let origin: String?
    let title: String
    let loadState: BrowserPageState
    let viewport: BrowserViewport
    let visibility: BrowserVisibility
    let revision: Int
    let ownership: BrowserOwnership
    let control: BrowserControlState
}

struct BrowserElement: Codable, Equatable, Sendable {
    let reference: String
    let role: String
    let accessibleName: String
    let state: [String: String]
    let bounds: BrowserRect
    let sensitive: Bool
    let submitsForm: Bool
}

struct BrowserFrame: Codable, Equatable, Sendable {
    let origin: String
    let bounds: BrowserRect
    let opaque: Bool
    let elements: [BrowserElement]
}

struct BrowserPageSnapshot: Codable, Equatable, Sendable {
    let revision: Int
    let url: String?
    let title: String
    let visibleText: String
    let headings: [String]
    let landmarks: [String]
    let forms: [String]
    let elements: [BrowserElement]
    let frames: [BrowserFrame]
    let viewport: BrowserViewport
    let bounded: Bool
}

struct BrowserAction: Codable, Equatable, Sendable {
    let type: String
    let reference: String?
    let revision: Int?
    let text: String?
    let key: String?
    let x: Double?
    let y: Double?
    let deltaX: Double?
    let deltaY: Double?

    enum CodingKeys: String, CodingKey {
        case type
        case reference = "ref"
        case revision
        case text
        case key
        case x
        case y
        case deltaX = "delta_x"
        case deltaY = "delta_y"
    }

    init(
        type: String,
        reference: String? = nil,
        revision: Int? = nil,
        text: String? = nil,
        key: String? = nil,
        x: Double? = nil,
        y: Double? = nil,
        deltaX: Double? = nil,
        deltaY: Double? = nil
    ) {
        self.type = type
        self.reference = reference
        self.revision = revision
        self.text = text
        self.key = key
        self.x = x
        self.y = y
        self.deltaX = deltaX
        self.deltaY = deltaY
    }

    func validationFailure(actionIndex: Int) -> BrowserFailure? {
        let expected = "Actions are objects. Semantic actions use {\"type\":\"click\",\"ref\":\"e_1_2\",\"revision\":1}; type adds text, press adds key, and scroll adds delta_x and/or delta_y. Coordinate click, hover, move, and scroll use x and y."
        func invalid(_ message: String) -> BrowserFailure {
            BrowserFailure(.invalidRequest, message, details: [
                "action_index": String(actionIndex),
                "expected": expected,
                "semantic_click_example": "{type: click, ref: e_1_2, revision: 1}",
                "coordinate_click_example": "{type: click, x: 120, y: 80}",
            ])
        }

        guard ["click", "type", "press", "hover", "move", "focus", "scroll"].contains(type) else {
            return invalid("page_interactions action type is unsupported.")
        }
        let hasReference = reference != nil || revision != nil
        let hasCoordinate = x != nil || y != nil
        guard hasReference != hasCoordinate else {
            return invalid("page_interactions action must use either ref plus revision or x plus y, but not both.")
        }
        if let revision, revision < 0 {
            return invalid("reference-based actions require a non-negative revision.")
        }
        for coordinate in [x, y] {
            if let coordinate, !coordinate.isFinite || coordinate < 0 {
                return invalid("coordinate actions require non-negative finite x and y values.")
            }
        }
        for delta in [deltaX, deltaY] {
            if let delta, !delta.isFinite {
                return invalid("scroll deltas must be finite numbers.")
            }
        }

        if hasReference {
            guard let reference, !reference.isEmpty, revision != nil else {
                return invalid("reference-based actions require a non-empty ref and an integer revision.")
            }
            guard x == nil, y == nil else {
                return invalid("reference-based actions cannot include x or y.")
            }
            switch type {
            case "click", "hover", "move", "focus":
                guard text == nil, key == nil, deltaX == nil, deltaY == nil else {
                    return invalid("This reference action accepts only type, ref, and revision.")
                }
            case "type":
                guard text != nil, key == nil, deltaX == nil, deltaY == nil else {
                    return invalid("type requires text and does not accept key or scroll deltas.")
                }
            case "press":
                guard key != nil, text == nil, deltaX == nil, deltaY == nil else {
                    return invalid("press requires key and does not accept text or scroll deltas.")
                }
            case "scroll":
                guard text == nil, key == nil, deltaX != nil || deltaY != nil else {
                    return invalid("scroll requires delta_x and/or delta_y and does not accept text or key.")
                }
            default:
                break
            }
        } else {
            guard x != nil, y != nil, reference == nil, revision == nil else {
                return invalid("coordinate actions require x and y and cannot include ref or revision.")
            }
            switch type {
            case "click", "hover", "move":
                guard text == nil, key == nil, deltaX == nil, deltaY == nil else {
                    return invalid("This coordinate action accepts only type, x, and y.")
                }
            case "scroll":
                guard text == nil, key == nil, deltaX != nil || deltaY != nil else {
                    return invalid("coordinate scroll requires delta_x and/or delta_y and does not accept text or key.")
                }
            default:
                return invalid("type, press, and focus require an Element Reference; coordinate actions support click, hover, move, and scroll.")
            }
        }
        return nil
    }
}

struct BrowserInteractionResult: Codable, Equatable, Sendable {
    let action: String
    let method: String
    let finalURL: String?
    let revision: Int
    let navigationBegan: Bool
    let control: BrowserControlState
}

struct BrowserRequestOptions: Codable, Equatable, Sendable {
    let timeoutMilliseconds: Int
    let wait: String

    init(timeoutMilliseconds: Int = 15_000, wait: String = "load") {
        self.timeoutMilliseconds = timeoutMilliseconds
        self.wait = wait
    }
}

enum BrowserJSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([BrowserJSONValue])
    case object([String: BrowserJSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([BrowserJSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: BrowserJSONValue].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }

    var stringValue: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    var intValue: Int? {
        guard case .number(let value) = self else { return nil }
        return Int(value)
    }

    var numberValue: Double? {
        guard case .number(let value) = self else { return nil }
        return value
    }

    var boolValue: Bool? {
        guard case .bool(let value) = self else { return nil }
        return value
    }

    var objectValue: [String: BrowserJSONValue]? {
        guard case .object(let value) = self else { return nil }
        return value
    }

    var arrayValue: [BrowserJSONValue]? {
        guard case .array(let value) = self else { return nil }
        return value
    }
}

struct BrowserRPCRequest: Codable, Sendable {
    let id: BrowserJSONValue
    let method: String
    let params: [String: BrowserJSONValue]
}

struct BrowserRPCResponse: Codable, Sendable {
    let id: BrowserJSONValue
    let result: BrowserJSONValue?
    let error: BrowserFailure?

    static func success(id: BrowserJSONValue, result: BrowserJSONValue) -> BrowserRPCResponse {
        BrowserRPCResponse(id: id, result: result, error: nil)
    }

    static func failure(id: BrowserJSONValue, _ failure: BrowserFailure) -> BrowserRPCResponse {
        BrowserRPCResponse(id: id, result: nil, error: failure)
    }
}

struct BrowserCue: Identifiable, Equatable, Sendable {
    let id: UUID
    let label: String
    let state: BrowserCueState
    let origin: String?
    /// The owning agent has driven this session within the activity window —
    /// the cue marks it, and ⌥-click refuses to destroy it.
    var isAgentActive: Bool = false
}

struct BrowserAuditEntry: Codable, Equatable, Sendable {
    let timestamp: Date
    let connectionLabel: String
    let tool: String
    let origin: String?
    let resultClass: String
}

enum BrowserSessionTransitionError: Error, Equatable {
    case invalid(String)
}

struct BrowserSessionStateMachine: Equatable, Sendable {
    private(set) var runtime: BrowserRuntimeState = .starting
    private(set) var page: BrowserPageState = .empty
    private(set) var visibility: BrowserVisibility = .hidden
    private(set) var ownership: BrowserOwnership = .connected
    private(set) var control: BrowserControlState = .agent

    mutating func ready() throws {
        guard runtime == .starting else { throw BrowserSessionTransitionError.invalid("ready") }
        runtime = .ready
    }

    mutating func beginLoading() throws {
        guard runtime == .ready else { throw BrowserSessionTransitionError.invalid("beginLoading") }
        page = .loading
    }

    mutating func interactive() throws {
        guard runtime == .ready, page == .loading else { throw BrowserSessionTransitionError.invalid("interactive") }
        page = .interactive
    }

    mutating func failPage() {
        page = .failed
    }

    mutating func setVisibility(_ value: BrowserVisibility) throws {
        guard runtime == .ready else { throw BrowserSessionTransitionError.invalid("visibility") }
        visibility = value
    }

    mutating func detach() throws {
        guard ownership == .connected else { throw BrowserSessionTransitionError.invalid("detach") }
        ownership = .detached
        control = .waitingForUser
    }

    mutating func requestReclaim() throws {
        guard ownership == .detached else { throw BrowserSessionTransitionError.invalid("requestReclaim") }
        ownership = .reclaimPending
        control = .waitingForUser
    }

    mutating func reclaim() throws {
        guard ownership == .reclaimPending else { throw BrowserSessionTransitionError.invalid("reclaim") }
        ownership = .connected
        control = .agent
    }

    mutating func cancelReclaim() throws {
        guard ownership == .reclaimPending else { throw BrowserSessionTransitionError.invalid("cancelReclaim") }
        ownership = .detached
        control = .waitingForUser
    }

    mutating func revokeControl() {
        control = .user
    }

    mutating func resumeAgent() throws {
        guard ownership == .connected else { throw BrowserSessionTransitionError.invalid("resumeAgent") }
        control = .agent
    }

    mutating func terminate() {
        runtime = .terminating
    }
}

struct BrowserSessionRegistry: Equatable, Sendable {
    let limit: Int
    private(set) var ownerByConnection: [String: UUID] = [:]

    init(limit: Int = 4) {
        self.limit = limit
    }

    mutating func claim(connectionID: String, sessionID: UUID) throws {
        if ownerByConnection[connectionID] != nil { return }
        guard ownerByConnection.count < limit else {
            throw BrowserFailure(BrowserErrorCode.sessionLimitReached, "The four live Browser Session limit is already in use.")
        }
        ownerByConnection[connectionID] = sessionID
    }

    mutating func release(connectionID: String) {
        ownerByConnection.removeValue(forKey: connectionID)
    }

    func session(for connectionID: String) -> UUID? {
        ownerByConnection[connectionID]
    }
}
