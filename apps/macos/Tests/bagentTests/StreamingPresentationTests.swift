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

    func testTextStorageUpdaterSurvivesStreamedMarkdownNormalizationAndFinalReplacement() {
        let storage = NSTextStorage()
        let streamed = """
        Read 2 of 3 emails · partial

        1. **Sender:** team@mail.perplexity.ai
           **Subject:** Product update
        2. [Sender](https://example.com): alerts@example.com
        """

        for end in streamed.indices {
            let source = String(streamed[...end])
            let expected = NotchMarkdown.attributedString(source)
            NotchTextStorageUpdater.apply(expected, to: storage)
            XCTAssertEqual(
                storage.string,
                expected.string,
                "Text storage diverged after streaming source prefix \(source.count)"
            )
        }

        // Evidence finalization may replace an intermediate draft with a
        // shorter canonical answer; this must reset rather than deriving a
        // replacement range from earlier rendered markdown.
        let finalized = NotchMarkdown.attributedString(
            "Read 2 of 3 emails · partial\n\n1. Sender: team@mail.perplexity.ai"
        )
        NotchTextStorageUpdater.apply(finalized, to: storage)
        XCTAssertEqual(storage.string, finalized.string)
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

    func testExpandedActivityTranscriptContributesToBridgeHeight() {
        XCTAssertEqual(
            NotchActivityLayout.extraHeight(activityCount: 0, expanded: true),
            0
        )
        XCTAssertEqual(
            NotchActivityLayout.extraHeight(activityCount: 2, expanded: false),
            NotchActivityLayout.headerHeight
        )
        XCTAssertEqual(
            NotchActivityLayout.extraHeight(activityCount: 2, expanded: true),
            NotchActivityLayout.headerHeight + 2 * NotchActivityLayout.rowHeight
        )
        XCTAssertEqual(
            NotchActivityLayout.extraHeight(activityCount: 20, expanded: true),
            NotchActivityLayout.headerHeight + NotchActivityLayout.maxRowsHeight
        )
    }
}
