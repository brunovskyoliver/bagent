import XCTest
import WebKit
@testable import bagent

final class BrowserNavigationPolicyTests: XCTestCase {
    private let resolver: BrowserNavigationPolicy.Resolver = { host in
        switch host {
        case "allowed.test": return ["172.19.4.10", "::1"]
        case "mixed.test": return ["172.19.4.10", "8.8.8.8"]
        case "public.test": return ["203.0.113.10"]
        case "empty.test": return []
        default: return []
        }
    }

    func testAllowlistAcceptsOnlyAgreedDirectAddresses() {
        let policy = BrowserNavigationPolicy(resolver: resolver)
        for raw in [
            "http://127.0.0.1/",
            "https://127.255.255.254/",
            "http://172.19.0.1/",
            "http://172.29.255.254/",
            "http://[::1]/",
            "https://allowed.test/",
        ] {
            XCTAssertTrue(policy.validate(URL(string: raw)!).isSuccess, raw)
        }
    }

    func testDisallowedRangeSchemeAndUnresolvedHostFailClosed() {
        let policy = BrowserNavigationPolicy(resolver: resolver)
        let cases: [(String, BrowserErrorCode)] = [
            ("http://172.20.0.1/", .navigationBlocked),
            ("https://public.test/", .navigationBlocked),
            ("file:///tmp/page.html", .navigationBlocked),
            ("javascript:alert(1)", .navigationBlocked),
            ("http://empty.test/", .navigationBlocked),
        ]
        for (raw, code) in cases {
            guard case .failure(let failure) = policy.validate(URL(string: raw)!) else {
                return XCTFail("Expected \(raw) to be blocked")
            }
            XCTAssertEqual(failure.code, code, raw)
        }
    }

    func testMixedDNSAnswersFailEvenWhenOneAnswerIsAllowed() {
        let policy = BrowserNavigationPolicy(resolver: resolver)
        guard case .failure(let failure) = policy.validate(URL(string: "http://mixed.test/")!) else {
            return XCTFail("Expected mixed DNS answer to be blocked")
        }
        XCTAssertEqual(failure.code, .mixedDNSAnswer)
    }

    func testOriginDropsQueryAndFragment() {
        let policy = BrowserNavigationPolicy(resolver: resolver)
        XCTAssertEqual(policy.origin(for: URL(string: "http://127.0.0.1:8080/path?q=secret#fragment")!), "http://127.0.0.1:8080")
    }
}

final class BrowserSessionStateTests: XCTestCase {
    func testRegistryEnforcesFourConnectionsAndImplicitOwnership() throws {
        var registry = BrowserSessionRegistry()
        for index in 0..<4 {
            try registry.claim(connectionID: "connection-\(index)", sessionID: UUID())
        }
        XCTAssertThrowsError(try registry.claim(connectionID: "connection-4", sessionID: UUID())) { error in
            XCTAssertEqual((error as? BrowserFailure)?.code, .sessionLimitReached)
        }
        XCTAssertNotNil(registry.session(for: "connection-2"))
        registry.release(connectionID: "connection-2")
        XCTAssertNil(registry.session(for: "connection-2"))
        try registry.claim(connectionID: "connection-4", sessionID: UUID())
    }

    func testStateMachineKeepsVisibilityOwnershipAndControlSeparate() throws {
        var state = BrowserSessionStateMachine()
        try state.ready()
        try state.setVisibility(.popup)
        try state.detach()
        XCTAssertEqual(state.visibility, .popup)
        XCTAssertEqual(state.ownership, .detached)
        XCTAssertEqual(state.control, .waitingForUser)
        try state.requestReclaim()
        try state.reclaim()
        XCTAssertEqual(state.ownership, .connected)
        XCTAssertEqual(state.control, .agent)
    }

    func testCanceledReclaimReturnsToDetachedState() throws {
        var state = BrowserSessionStateMachine()
        try state.ready()
        try state.detach()
        try state.requestReclaim()
        try state.cancelReclaim()
        XCTAssertEqual(state.ownership, .detached)
        XCTAssertEqual(state.control, .waitingForUser)
    }

    func testDirectInputRevocationRequiresExplicitResume() throws {
        var state = BrowserSessionStateMachine()
        try state.ready()
        state.revokeControl()
        XCTAssertEqual(state.control, .user)
        XCTAssertEqual(state.control, .user)
        try state.resumeAgent()
        XCTAssertEqual(state.control, .agent)
    }
}

@MainActor
final class BrowserVisibilityRegressionTests: XCTestCase {
    func testBrowserOpenCannotRevealThePanel() async {
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: true)
        let response = await coordinator.handle(
            connectionID: "open-visibility-test",
            connectionLabel: "open-visibility-test",
            request: BrowserRPCRequest(
                id: .string("open"),
                method: "browser_open",
                params: ["visible": .bool(true)]
            )
        )

        XCTAssertEqual(response.error?.code, .invalidRequest)
        XCTAssertEqual(coordinator.sessions.values.first?.pageInfo.visibility, .hidden)
        XCTAssertFalse(coordinator.sessions.values.first?.windowController.panel.isVisible ?? true)
    }

    func testDisabledBrowserReturnsStructuredErrorWithoutCreatingASession() async {
        let coordinator = BrowserCoordinator(profile: BrowserProfile(identifier: UUID()), enabled: false)
        let response = await coordinator.handle(
            connectionID: "disabled-browser-test",
            connectionLabel: "disabled-browser-test",
            request: BrowserRPCRequest(id: .string("open"), method: "browser_open", params: [:])
        )

        XCTAssertEqual(response.error?.code, .browserDisabled)
        XCTAssertTrue(coordinator.sessions.isEmpty)
        XCTAssertTrue(response.error?.message.contains("Enable bagent Browser in Settings") == true)
    }

    func testHiddenSemanticClickStaysHidden() async throws {
        let profile = BrowserProfile(identifier: UUID())
        let session = BrowserSession(
            ownerConnectionID: "visibility-test",
            ownerLabel: "visibility-test",
            profile: profile
        )
        session.webView.loadHTMLString(
            "<main><p id='status'>Ready</p><button id='status-button' onclick=\"document.querySelector('#status').textContent='Clicked'\">Update status</button></main>",
            baseURL: URL(string: "http://127.0.0.1:8765/browser.html")
        )

        var snapshot: BrowserPageSnapshot?
        for _ in 0..<40 {
            snapshot = try? await session.snapshot()
            if snapshot?.elements.contains(where: { $0.accessibleName == "Update status" }) == true { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard let snapshot, let button = snapshot.elements.first(where: { $0.accessibleName == "Update status" }) else {
            return XCTFail("The semantic fixture button did not become available")
        }

        XCTAssertEqual(session.pageInfo.visibility, .hidden)
        XCTAssertFalse(session.windowController.panel.isVisible)
        let result = try await session.interact(BrowserAction(type: "click", reference: button.reference, revision: snapshot.revision))

        XCTAssertEqual(result.method, "dom")
        XCTAssertEqual(session.pageInfo.visibility, .hidden)
        XCTAssertFalse(session.windowController.panel.isVisible)
    }

    func testPageSnapshotMarksPasswordFieldsWithoutIncludingTheirValue() async throws {
        let profile = BrowserProfile(identifier: UUID())
        let session = BrowserSession(
            ownerConnectionID: "sensitive-data-test",
            ownerLabel: "sensitive-data-test",
            profile: profile
        )
        session.webView.loadHTMLString(
            "<main><label>Password <input type='password' value='fixture-secret' aria-label='Password'></label></main>",
            baseURL: URL(string: "http://127.0.0.1:8765/browser.html")
        )

        var snapshot: BrowserPageSnapshot?
        for _ in 0..<40 {
            snapshot = try? await session.snapshot()
            if snapshot?.elements.contains(where: { $0.sensitive }) == true { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard let snapshot, let password = snapshot.elements.first(where: { $0.sensitive }) else {
            return XCTFail("The password field did not become available")
        }

        XCTAssertEqual(password.role, "textbox")
        let encoded = String(decoding: try JSONEncoder().encode(snapshot), as: UTF8.self)
        XCTAssertFalse(encoded.contains("fixture-secret"))
    }
}

@MainActor
final class BrowserProfileTests: XCTestCase {
    func testProfileClearRemovesWebsiteDataWithoutTouchingSafari() async {
        let profile = BrowserProfile(identifier: UUID())
        let cookie = HTTPCookie(properties: [
            .domain: "127.0.0.1",
            .path: "/",
            .name: "browser_profile_test",
            .value: "present",
        ])!
        await withCheckedContinuation { continuation in
            profile.dataStore.httpCookieStore.setCookie(cookie) {
                continuation.resume()
            }
        }
        let before = await withCheckedContinuation { continuation in
            profile.dataStore.httpCookieStore.getAllCookies { cookies in
                continuation.resume(returning: cookies.contains { $0.name == "browser_profile_test" })
            }
        }
        XCTAssertTrue(before)

        await profile.clear()

        let after = await withCheckedContinuation { continuation in
            profile.dataStore.httpCookieStore.getAllCookies { cookies in
                continuation.resume(returning: cookies.contains { $0.name == "browser_profile_test" })
            }
        }
        XCTAssertFalse(after)
        XCTAssertTrue(profile.isPersistent)
    }
}

private extension Result where Failure == BrowserFailure {
    var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}
