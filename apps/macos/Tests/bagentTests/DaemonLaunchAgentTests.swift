import XCTest
@testable import bagent

final class DaemonLaunchAgentTests: XCTestCase {
    func testRuntimeEnvironmentDropsRemovedAuthoritySwitch() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "0",
        ])
        XCTAssertNil(environment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
        XCTAssertNil(environment["BAGENT_SYNTHESIS_FALLBACK_MODEL_PATH"])
        XCTAssertNotNil(environment["BAGENT_CHAT_MODEL_PATH"])
    }

    func testResolverModeIsForwardedVerbatimWithoutATopLevelSwitch() {
        // Typed evidence routing is unconditional after Stage 8, so the resolver
        // mode is an independent setting with no higher-precedence flag above it.
        for value in ["enforce", " observe ", "persistence", "Enforce & keep"] {
            let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
                "BAGENT_REFERENCE_RESOLVER_MODE": value,
            ])
            XCTAssertEqual(environment["BAGENT_REFERENCE_RESOLVER_MODE"], value)
            XCTAssertNil(environment["BAGENT_EVIDENCE_ORCHESTRATOR"])
        }
    }

    func testAbsentSubordinateResolverModeAddsNothing() {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [:])

        XCTAssertNil(environment["BAGENT_REFERENCE_RESOLVER_MODE"])
        XCTAssertNil(environment["BAGENT_EVIDENCE_ORCHESTRATOR"])
        XCTAssertNil(environment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
    }

    func testAcceptanceFixtureSwitchStillForwards() {
        let acceptance = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "1",
        ])
        XCTAssertEqual(acceptance["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"], "1")
    }

    func testPlistPreservesSelectedResolverEnvironmentExactly() throws {
        let environment = DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
            "BAGENT_REFERENCE_RESOLVER_MODE": "Enforce & keep",
        ])
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: environment
        )
        let parsed = try PropertyListSerialization.propertyList(
            from: Data(plist.utf8), options: [], format: nil
        ) as? [String: Any]
        let serialized = parsed?["EnvironmentVariables"] as? [String: String]

        XCTAssertEqual(serialized?["BAGENT_REFERENCE_RESOLVER_MODE"], "Enforce & keep")
        XCTAssertNil(serialized?["BAGENT_EVIDENCE_ORCHESTRATOR"])
    }

    func testPlistContainsBinaryLabelAndSortedEnv() throws {
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: [
                "BAGENT_BASERT_BASE_URL": "http://127.0.0.1:8082/v1",
                "BAGENT_DEFAULT_MODEL": ModelRuntimeConfiguration.model,
            ]
        )
        XCTAssertTrue(plist.contains("<string>com.bagent.daemon</string>"))
        XCTAssertTrue(plist.contains("<string>/Applications/bagent.app/Contents/MacOS/bagentd</string>"))
        // Env is emitted sorted by key so identical settings produce identical plists.
        let defaultRange = plist.range(of: "BAGENT_DEFAULT_MODEL")!
        let baseURLRange = plist.range(of: "BAGENT_BASERT_BASE_URL")!
        XCTAssertLessThan(baseURLRange.lowerBound, defaultRange.lowerBound)
        // Crash-only KeepAlive: clean exits stay down.
        XCTAssertTrue(plist.contains("<key>SuccessfulExit</key>"))
        XCTAssertTrue(plist.contains("<key>RunAtLoad</key>"))
        // Valid XML plist.
        let parsed = try PropertyListSerialization.propertyList(
            from: plist.data(using: .utf8)!, options: [], format: nil) as? [String: Any]
        XCTAssertEqual(parsed?["Label"] as? String, "com.bagent.daemon")
    }

    func testPlistDoesNotAddRemovedAuthoritySwitch() throws {
        let plist = DaemonLaunchAgent.plistContent(
            binaryPath: "/Applications/bagent.app/Contents/MacOS/bagentd",
            environment: DaemonLaunchAgent.runtimeEnvironment(processEnvironment: [
                "BAGENT_STAGE8_ACCEPTANCE_FIXTURES": "0",
            ])
        )
        let parsed = try PropertyListSerialization.propertyList(
            from: Data(plist.utf8), options: [], format: nil
        ) as? [String: Any]
        let environment = parsed?["EnvironmentVariables"] as? [String: String]
        XCTAssertNil(environment?["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"])
    }
}
