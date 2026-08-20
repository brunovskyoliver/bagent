import Foundation
import SwiftUI

private struct CompassRailHighContrastKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var compassRailHighContrast: Bool {
        get { self[CompassRailHighContrastKey.self] }
        set { self[CompassRailHighContrastKey.self] = newValue }
    }
}

enum CompassRailArea: String, CaseIterable, Hashable, Sendable {
    case general
    case modelRuntime = "model_runtime"
    case integrations
    case privacyAndPermissions = "privacy_and_permissions"

    var title: String {
        switch self {
        case .general: String(localized: "settings.area.general", defaultValue: "General")
        case .modelRuntime: String(localized: "settings.area.modelRuntime", defaultValue: "Model and Runtime")
        case .integrations: String(localized: "settings.area.integrations", defaultValue: "Integrations")
        case .privacyAndPermissions: String(localized: "settings.area.privacy", defaultValue: "Privacy and Permissions")
        }
    }

    var backAccessibilityLabel: String {
        switch self {
        case .general: String(localized: "Back to General", defaultValue: "Back to General")
        case .modelRuntime: String(localized: "Back to Model and Runtime", defaultValue: "Back to Model and Runtime")
        case .integrations: String(localized: "Back to Integrations", defaultValue: "Back to Integrations")
        case .privacyAndPermissions: String(localized: "Back to Privacy and Permissions", defaultValue: "Back to Privacy and Permissions")
        }
    }

    var symbolName: String {
        switch self {
        case .general: "gearshape"
        case .modelRuntime: "cpu"
        case .integrations: "puzzlepiece.extension"
        case .privacyAndPermissions: "lock.shield"
        }
    }

    var index: Int { Self.allCases.firstIndex(of: self) ?? 0 }
}

enum CompassRailChild: String, CaseIterable, Hashable, Sendable {
    case whatsapp
    case odoo
    case codex
    case fullDiskAccess = "full_disk_access"
    case screenRecording = "screen_recording"
    case accessibility
    case rulesAndApprovalPolicy = "rules_and_approval_policy"

    var parent: CompassRailArea {
        switch self {
        case .whatsapp, .odoo, .codex: .integrations
        case .fullDiskAccess, .screenRecording, .accessibility, .rulesAndApprovalPolicy: .privacyAndPermissions
        }
    }

    var title: String {
        switch self {
        case .whatsapp: String(localized: "settings.child.whatsapp", defaultValue: "WhatsApp")
        case .odoo: String(localized: "settings.child.odoo", defaultValue: "Odoo")
        case .codex: String(localized: "settings.child.codex", defaultValue: "Codex")
        case .fullDiskAccess: String(localized: "settings.child.fullDiskAccess", defaultValue: "Full Disk Access")
        case .screenRecording: String(localized: "settings.child.screenRecording", defaultValue: "Screen Recording")
        case .accessibility: String(localized: "settings.child.accessibility", defaultValue: "Accessibility")
        case .rulesAndApprovalPolicy: String(localized: "settings.child.rules", defaultValue: "Rules and approval policy")
        }
    }
}

enum CompassRailRoute: Hashable, Sendable {
    case area(CompassRailArea)
    case child(CompassRailChild)

    static let initial: Self = .area(.general)

    /// The deterministic state catalog used by tests and the disposable fixture.
    static let acceptedRoutes: [Self] = [
        .area(.general),
        .area(.modelRuntime),
        .area(.integrations),
        .child(.whatsapp),
        .child(.odoo),
        .child(.codex),
        .area(.privacyAndPermissions),
        .child(.fullDiskAccess),
        .child(.screenRecording),
        .child(.accessibility),
        .child(.rulesAndApprovalPolicy)
    ]

    var area: CompassRailArea {
        switch self {
        case .area(let area): area
        case .child(let child): child.parent
        }
    }

    var child: CompassRailChild? {
        if case .child(let child) = self { return child }
        return nil
    }

    var isChild: Bool { child != nil }

    var identifier: String {
        switch self {
        case .area(let area): "area-\(area.rawValue)"
        case .child(let child): "child-\(child.rawValue)"
        }
    }

    var parent: Self? {
        guard case .child(let child) = self else { return nil }
        return .area(child.parent)
    }

    func moving(_ direction: CompassRailDirection) -> Self {
        let offset = direction == .left ? -1 : 1
        let count = CompassRailArea.allCases.count
        let nextIndex = (area.index + offset + count) % count
        return .area(CompassRailArea.allCases[nextIndex])
    }
}

enum CompassRailDirection: Equatable, Sendable {
    case left
    case right
}

struct CompassRailSelection: Equatable, Sendable {
    private(set) var route: CompassRailRoute = .initial

    init(route: CompassRailRoute = .initial) {
        self.route = route
    }

    var area: CompassRailArea { route.area }

    mutating func select(_ area: CompassRailArea) {
        route = .area(area)
    }

    mutating func selectChild(_ child: CompassRailChild) {
        route = .child(child)
    }

    mutating func goBack() -> CompassRailRoute? {
        guard let parent = route.parent else { return nil }
        route = parent
        return parent
    }

}

enum CompassRailFocusedControl: Hashable, Sendable {
    case rail(CompassRailArea)
    case child(CompassRailChild)
    case appleMail
    case appleNotes
    case textField
    case secureField
    case textEditor
    case picker
    case slider
    case nativeEditor

    var yieldsArrowNavigation: Bool {
        switch self {
        case .textField, .secureField, .textEditor, .picker, .slider, .nativeEditor: true
        case .rail, .child, .appleMail, .appleNotes: false
        }
    }
}

enum CompassRailKey: Equatable, Sendable {
    case left
    case right
    case escape
}

enum CompassRailKeyboardAction: Equatable, Sendable {
    case select(CompassRailArea)
    case back
    case collapse
}

enum CompassRailKeyboard {
    static func route(
        _ key: CompassRailKey,
        route: CompassRailRoute,
        focusedControl: CompassRailFocusedControl?
    ) -> CompassRailKeyboardAction? {
        if focusedControl?.yieldsArrowNavigation == true,
           key == .left || key == .right {
            return nil
        }
        if route.isChild, key == .left || key == .right { return nil }
        switch key {
        case .left: return .select(route.moving(.left).area)
        case .right: return .select(route.moving(.right).area)
        case .escape: return route.isChild ? .back : .collapse
        }
    }
}

struct CompassRailFocusMemory: Equatable, Sendable {
    private(set) var openingControl: CompassRailFocusedControl?

    mutating func remember(_ control: CompassRailFocusedControl) {
        openingControl = control
    }

    func controlToRestore(afterOpening _: CompassRailChild) -> CompassRailFocusedControl? { openingControl }
}

enum CompassRailPreemption: Equatable, Sendable {
    case approval
    case whatsappPairing
    case settings

    static func resolve(approvalPending: Bool, whatsappPairing: Bool) -> Self {
        if approvalPending { return .approval }
        if whatsappPairing { return .whatsappPairing }
        return .settings
    }
}

enum CompassRailStateCatalog {
    static let routes = CompassRailRoute.acceptedRoutes
    static let modelRuntimeStates = NotchModelPhase.allCases
    static let modelRuntimeFixtureStates = ["baseline", "active-lease", "waiting-for-residency", "changed-pid-recovery"]
    static let validationStates = [
        "Not tested", "Testing", "Valid", "Validation failed", "Local service unavailable"
    ]
    static let permissionStates = ["Needs setup", "Active"]
    static let syntheticPanelWidths: [Double] = [701, 941]
    static let highContrastSecondaryOpacity = 0.80
}

struct CompassRailAcceptanceState: Equatable, Sendable {
    let selectedModel: String
    let modelPhase: NotchModelPhase
    let validationState: String
    let permissionGranted: Bool
    let preloadOnInput: Bool
    let idleTimeoutSeconds: Int
    let activeLease: Bool
    let automationWaiting: Bool
    let changedPIDRecovery: String

    init(
        modelPhase: NotchModelPhase,
        validationState: String,
        permissionGranted: Bool,
        preloadOnInput: Bool,
        idleTimeoutSeconds: Int,
        activeLease: Bool = false,
        automationWaiting: Bool = false,
        changedPIDRecovery: String = "Not reported",
        selectedModel: String = "basecompute/Chat4B"
    ) {
        self.selectedModel = selectedModel
        self.modelPhase = modelPhase
        self.validationState = validationState
        self.permissionGranted = permissionGranted
        self.preloadOnInput = preloadOnInput
        self.idleTimeoutSeconds = idleTimeoutSeconds
        self.activeLease = activeLease
        self.automationWaiting = automationWaiting
        self.changedPIDRecovery = changedPIDRecovery
    }

    static let baseline = Self(modelPhase: .ready, validationState: "Not tested", permissionGranted: false, preloadOnInput: false, idleTimeoutSeconds: 1200)
}
