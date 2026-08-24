import XCTest
@testable import bagent

final class AppLaunchModeTests: XCTestCase {
    func testOnlyOpaqueTokenIsAcceptedForUIOnlyRelaunch() {
        XCTAssertEqual(
            AppLaunchMode.parse(arguments: ["bagent", "--ui-relaunch-token", "opaque-123"]),
            .uiOnlyRelaunch(token: "opaque-123")
        )
        XCTAssertFalse(AppLaunchMode.parse(arguments: ["bagent", "--ui-relaunch-token", "opaque-123"]).startsDaemon)
        XCTAssertTrue(AppLaunchMode.parse(arguments: ["bagent"]).startsDaemon)
        XCTAssertEqual(
            AppLaunchMode.parse(arguments: ["bagent", "--ui-relaunch-token", "", "extra"]),
            .invalidUIOnlyRelaunch
        )
        XCTAssertFalse(AppLaunchMode.parse(arguments: ["bagent", "--ui-relaunch-token"]).startsDaemon)
        XCTAssertFalse(AppLaunchMode.parse(arguments: ["bagent", "--ui-relaunch-token"]).startsMonitoring)
    }
}
