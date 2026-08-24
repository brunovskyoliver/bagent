import XCTest
@testable import bagent

final class PermissionRoutingTests: XCTestCase {
    func testRoutesUseTheAcceptedCategoryAnchors() {
        XCTAssertEqual(
            PermissionSystemSettingsRoute.destination(for: .privacyAndSecurityRoot).absoluteString,
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension"
        )
        XCTAssertEqual(
            PermissionSystemSettingsRoute.destination(for: .fullDiskAccess).absoluteString,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
        )
        XCTAssertEqual(
            PermissionSystemSettingsRoute.destination(for: .screenRecording).absoluteString,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        )
        XCTAssertEqual(
            PermissionSystemSettingsRoute.destination(for: .accessibility).absoluteString,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        )
    }

    func testUnconfirmedCategoryFallsBackToPrivacyRootAndNeverGrants() {
        let opener = StubPermissionSettingsOpener(confirm: false)
        let result = PermissionSettingsRouter.open(.screenRecording, opener: opener)

        XCTAssertEqual(result, .fallbackToRoot(.screenRecording))
        XCTAssertEqual(opener.opened, [
            PermissionSystemSettingsRoute.destination(for: .screenRecording),
            PermissionSystemSettingsRoute.destination(for: .privacyAndSecurityRoot)
        ])
        XCTAssertFalse(result.establishesPermission)
    }

    func testConfirmedCategoryDoesNotUseRootFallback() {
        let opener = StubPermissionSettingsOpener(confirm: true)
        let result = PermissionSettingsRouter.open(.accessibility, opener: opener)

        XCTAssertEqual(result, .openedExact(.accessibility))
        XCTAssertEqual(opener.opened, [
            PermissionSystemSettingsRoute.destination(for: .accessibility)
        ])
        XCTAssertFalse(result.establishesPermission)
    }

    func testExpectedTitlesAreLocalizedByPermissionAndSupportedOS() {
        XCTAssertEqual(
            PermissionSystemSettingsRoute.expectedTitles(for: .screenRecording, osMajor: 26),
            ["Screen & System Audio Recording"]
        )
        XCTAssertEqual(
            PermissionSystemSettingsRoute.expectedTitles(for: .screenRecording, osMajor: 14),
            ["Screen Recording", "Screen & System Audio Recording"]
        )
        XCTAssertEqual(
            PermissionSystemSettingsRoute.expectedTitles(for: .privacyAndSecurityRoot, osMajor: 15),
            ["Privacy & Security"]
        )
    }

    func testStaleConfirmationCannotReplaceAnewerRouteAttempt() {
        let stale = PermissionSettingsRouter.confirmation(
            requested: .fullDiskAccess,
            observedTitle: "Accessibility"
        )
        XCTAssertEqual(stale, .unconfirmed)
    }
}

private final class StubPermissionSettingsOpener: PermissionSettingsOpener {
    let confirm: Bool
    private(set) var opened: [URL] = []

    init(confirm: Bool) {
        self.confirm = confirm
    }

    func open(_ url: URL) -> Bool {
        opened.append(url)
        return true
    }

    func confirmsExpectedPane(_ route: PermissionSystemSettingsRoute) -> Bool {
        confirm && route != .privacyAndSecurityRoot
    }
}
