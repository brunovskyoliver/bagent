import AppKit
import XCTest
@testable import bagent

final class StreamingPresentationTests: XCTestCase {
    @MainActor
    func testAdaptivePresenterPreservesCanonicalTextAcrossProviderChunks() async {
        var displayed = ""
        let presenter = AdaptiveStreamPresenter { displayed += $0 }
        for chunk in ["Dob", "rý ", "deň", ", ", "svet", "!\n", "Ďakujem."] {
            presenter.enqueue(chunk)
        }
        await presenter.finish()
        XCTAssertEqual(displayed, "Dobrý deň, svet!\nĎakujem.")
    }

    @MainActor
    func testAdaptivePresenterPreservesEmojiGraphemes() async {
        var displayed = ""
        let presenter = AdaptiveStreamPresenter { displayed += $0 }
        presenter.enqueue("Ahoj 👨‍👩‍👧‍👦 ")
        presenter.enqueue("svet")
        await presenter.finish()
        XCTAssertEqual(displayed, "Ahoj 👨‍👩‍👧‍👦 svet")
    }

    func testMarkdownValidatesLinksAndDetectsBareURLs() {
        let rendered = NotchMarkdown.attributedString(
            "[Safe](https://example.com) [Bad](javascript:alert(1)) https://swift.org"
        )
        var links: [URL] = []
        rendered.enumerateAttribute(
            .link,
            in: NSRange(location: 0, length: rendered.length)
        ) { value, _, _ in
            if let url = value as? URL { links.append(url) }
            if let raw = value as? String, let url = URL(string: raw) { links.append(url) }
        }
        XCTAssertEqual(Set(links.map(\.absoluteString)), [
            "https://example.com",
            "https://swift.org",
        ])
        XCTAssertTrue(rendered.string.contains("[Bad](javascript:alert(1))"))
    }

    func testMarkdownRendersFencedCodeAndBlockquote() {
        let rendered = NotchMarkdown.attributedString(
            "> quoted\n```swift\nlet value = 1\n```"
        )
        XCTAssertTrue(rendered.string.contains("▎ quoted"))
        XCTAssertTrue(rendered.string.contains("let value = 1"))
        XCTAssertFalse(rendered.string.contains("```"))
    }

    func testOutputStatusDotStaysAtTopRightInsteadOfBridgeCenter() {
        let point = NotchStatusDotGeometry.outputTopRight(
            notchOffset: 260,
            notchWidth: 221,
            targetWingWidth: 154,
            notchHeight: 39
        )
        XCTAssertEqual(point.y, 51)
        XCTAssertGreaterThan(point.x, 260 + 221)
        XCTAssertLessThan(point.y, 39 + 96 / 2)
    }
}
