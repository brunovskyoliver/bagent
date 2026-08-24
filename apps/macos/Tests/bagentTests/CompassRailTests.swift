import XCTest
@testable import bagent

final class CompassRailTests: XCTestCase {
    func testAcceptedPeersAndRoutesAreClosedAndOrdered() {
        XCTAssertEqual(
            CompassRailArea.allCases,
            [.general, .modelRuntime, .integrations, .privacyAndPermissions]
        )
        XCTAssertEqual(
            CompassRailRoute.acceptedRoutes,
            [
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
        )
        XCTAssertFalse(CompassRailArea.allCases.contains { $0.rawValue == "setup" })
    }

    func testSettingsOpensGeneralAndSelectionPersistsAcrossProjectionUpdates() {
        var selection = CompassRailSelection()
        XCTAssertEqual(selection.route, .area(.general))

        selection.select(.integrations)
        XCTAssertEqual(selection.route, .area(.integrations))
        selection.selectChild(.whatsapp)
        XCTAssertEqual(selection.route, .child(.whatsapp))
        XCTAssertEqual(selection.route, .child(.whatsapp))
    }

    @MainActor
    func testChatViewModelSelectionSurvivesAuthoritativeProjectionUpdate() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.openNotchSettings()
        viewModel.selectCompassRailArea(.integrations)
        try viewModel.installThinkingFixture()
        XCTAssertEqual(viewModel.compassRailRoute, .area(.integrations))
    }

    func testRailSelectionFromChildChangesTopLevelAndBackRestoresParent() {
        var selection = CompassRailSelection(route: .child(.odoo))
        XCTAssertEqual(selection.area, .integrations)

        selection.select(.privacyAndPermissions)
        XCTAssertEqual(selection.route, .area(.privacyAndPermissions))

        selection = CompassRailSelection(route: .child(.rulesAndApprovalPolicy))
        XCTAssertEqual(selection.goBack(), .area(.privacyAndPermissions))
        XCTAssertEqual(selection.goBack(), nil)
    }

    func testLeftAndRightWrapExactlyAcrossFourPeers() {
        XCTAssertEqual(CompassRailRoute.area(.general).moving(.left), .area(.privacyAndPermissions))
        XCTAssertEqual(CompassRailRoute.area(.privacyAndPermissions).moving(.right), .area(.general))
        XCTAssertEqual(CompassRailRoute.child(.whatsapp).moving(.left), .area(.modelRuntime))
        XCTAssertEqual(CompassRailRoute.child(.rulesAndApprovalPolicy).moving(.right), .area(.general))
    }

    func testEditableControlsYieldPlainLeftAndRight() {
        XCTAssertNil(CompassRailKeyboard.route(.left, route: .area(.integrations), focusedControl: .textEditor))
        XCTAssertNil(CompassRailKeyboard.route(.right, route: .area(.integrations), focusedControl: .picker))
        XCTAssertEqual(
            CompassRailKeyboard.route(.right, route: .area(.general), focusedControl: nil),
            .select(.modelRuntime)
        )
        XCTAssertNil(CompassRailKeyboard.route(.left, route: .child(.odoo), focusedControl: .rail(.integrations)))
    }

    func testBackAndEscapeAreOneLevelActions() {
        XCTAssertEqual(
            CompassRailKeyboard.route(.escape, route: .child(.codex), focusedControl: nil),
            .back
        )
        XCTAssertEqual(
            CompassRailKeyboard.route(.escape, route: .area(.general), focusedControl: nil),
            .collapse
        )
    }

    func testFocusRestorationRemembersOpeningControl() {
        var focus = CompassRailFocusMemory()
        focus.remember(.rail(.integrations))
        XCTAssertEqual(focus.controlToRestore(afterOpening: .odoo), .rail(.integrations))
        focus.remember(.child(.odoo))
        XCTAssertEqual(focus.controlToRestore(afterOpening: .odoo), .child(.odoo))
        XCTAssertEqual(focus.controlToRestore(afterOpening: .rulesAndApprovalPolicy), .child(.odoo))
    }

    func testPreemptionHasPriorityOverSettingsRoute() {
        XCTAssertEqual(
            CompassRailPreemption.resolve(approvalPending: true, whatsappPairing: false),
            .approval
        )
        XCTAssertEqual(
            CompassRailPreemption.resolve(approvalPending: false, whatsappPairing: true),
            .whatsappPairing
        )
        XCTAssertEqual(
            CompassRailPreemption.resolve(approvalPending: false, whatsappPairing: false),
            .settings
        )
    }

    func testSettingsGeometryAndStatusPillAreFixedForSyntheticAndWideNotches() {
        XCTAssertEqual(NotchWrapMetrics.settingsWingWidth, 205)
        XCTAssertEqual(NotchWrapMetrics.settingsBridgeHeight, 252)
        XCTAssertEqual(NotchPillLayout.size, CGSize(width: 74, height: 18))
        XCTAssertEqual(NotchPillLayout.settingsOrigin(maxPanelWidth: 701), CGPoint(x: 560, y: 9))
        XCTAssertEqual(NotchPillLayout.settingsOrigin(maxPanelWidth: 941), CGPoint(x: 800, y: 9))
    }

    func testDeterministicSettingsStateCatalogHasEveryStage7BState() {
        XCTAssertEqual(CompassRailStateCatalog.routes.count, 11)
        XCTAssertEqual(CompassRailStateCatalog.routes.first, .area(.general))
        XCTAssertEqual(CompassRailStateCatalog.routes.last, .child(.rulesAndApprovalPolicy))
        XCTAssertEqual(Set(CompassRailStateCatalog.routes.map(\.area)), Set(CompassRailArea.allCases))
        XCTAssertEqual(CompassRailStateCatalog.modelRuntimeStates, NotchModelPhase.allCases)
        XCTAssertEqual(CompassRailStateCatalog.validationStates.count, 5)
        XCTAssertEqual(CompassRailStateCatalog.permissionStates, ["Needs setup", "Active"])
        XCTAssertEqual(CompassRailStateCatalog.syntheticPanelWidths, [701, 941])
    }
}
