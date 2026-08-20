import XCTest
@testable import bagent

final class CompassRailAccessibilityTests: XCTestCase {
    func testAccessibilityCatalogIncludesEveryRouteAndReducedMotionContract() {
        XCTAssertEqual(CompassRailStateCatalog.routes.count, 11)
        XCTAssertEqual(CompassRailArea.allCases.count, 4)
        XCTAssertEqual(CompassRailChild.allCases.count, 7)
        XCTAssertEqual(CompassRailStateCatalog.modelRuntimeStates.count, 8)
        XCTAssertEqual(CompassRailStateCatalog.validationStates.count, 5)
        XCTAssertEqual(CompassRailStateCatalog.permissionStates.count, 2)
        XCTAssertEqual(NotchWrapMetrics.settingsWingWidth, 205)
        XCTAssertEqual(NotchWrapMetrics.settingsBridgeHeight, 252)
    }
}
