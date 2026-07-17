import XCTest
@testable import bagent

final class SlashCommandRegistryTests: XCTestCase {
    func testEmptyAndOrdinaryInputHaveNoSuggestions() {
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "").isEmpty)
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "hello").isEmpty)
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "nájdi faktúru s DPH").isEmpty)
        // Whitespace after the command word means an ordinary prompt.
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "/settings please").isEmpty)
    }

    func testPrefixFilteringIsCaseInsensitive() {
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/").map(\.id), ["settings", "automations"])
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/s").map(\.id), ["settings"])
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/S").map(\.id), ["settings"])
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/SETT").map(\.id), ["settings"])
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "/x").isEmpty)
    }

    func testAliasPrefixMatches() {
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/nasta").map(\.id), ["settings"])
    }

    func testAtMostThreeSuggestions() {
        XCTAssertLessThanOrEqual(SlashCommandRegistry.suggestions(for: "/").count, 3)
        XCTAssertEqual(SlashCommandRegistry.maxSuggestions, 3)
    }

    func testExactMatchUsesCanonicalAndAliases() {
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/settings")?.id, "settings")
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/SETTINGS")?.id, "settings")
        XCTAssertEqual(SlashCommandRegistry.exactMatch("  /nastavenia ")?.id, "settings")
        XCTAssertNil(SlashCommandRegistry.exactMatch("/sett"))     // incomplete stays editable
        XCTAssertNil(SlashCommandRegistry.exactMatch("/unknown"))  // unknown submits normally
        XCTAssertNil(SlashCommandRegistry.exactMatch("settings"))  // no slash → ordinary prompt
    }

    func testCanonicalSpellingIsLowercase() {
        for cmd in SlashCommandRegistry.all {
            XCTAssertEqual(cmd.command, cmd.command.lowercased())
            XCTAssertTrue(cmd.command.hasPrefix("/"))
        }
    }

    func testAutomationsRegisteredWithSurface() {
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/automations")?.id, "automations")
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/automatizácie")?.id, "automations")
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/a").map(\.id), ["automations"])
        // "/s" still uniquely matches settings.
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/s").map(\.id), ["settings"])
    }
}

@MainActor
final class SlashSuggestionStateTests: XCTestCase {
    func testTypingUpdatesSuggestionsAndSelectionWraps() {
        let vm = ChatViewModel()
        vm.inputText = "/s"
        XCTAssertEqual(vm.slashSuggestions.map(\.id), ["settings"])
        XCTAssertEqual(vm.slashSelectionIndex, 0)

        // Single row: up/down wrap onto the same row and stay consumed.
        XCTAssertTrue(vm.moveSlashSelection(by: 1))
        XCTAssertEqual(vm.slashSelectionIndex, 0)
        XCTAssertTrue(vm.moveSlashSelection(by: -1))
        XCTAssertEqual(vm.slashSelectionIndex, 0)

        // No suggestions → arrows are not consumed.
        vm.inputText = "plain prompt"
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
        XCTAssertFalse(vm.moveSlashSelection(by: 1))
    }

    func testEscapeDismissesUntilTextChanges() {
        let vm = ChatViewModel()
        vm.inputText = "/s"
        XCTAssertFalse(vm.slashSuggestions.isEmpty)
        vm.dismissSlashSuggestions()
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
        // Text unchanged → stays dismissed; typing re-evaluates.
        vm.inputText = "/se"
        XCTAssertFalse(vm.slashSuggestions.isEmpty)
    }

    func testAcceptExecutesSettingsWithCanonicalSpelling() {
        let vm = ChatViewModel()
        vm.inputText = "/SE"
        XCTAssertTrue(vm.acceptSlashSuggestion())
        // /settings opened the settings surface and cleared the input.
        XCTAssertEqual(vm.notchInteractionMode, .settings)
        XCTAssertEqual(vm.inputText, "")
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
    }

    func testUnknownSlashTextStaysEditable() {
        let vm = ChatViewModel()
        vm.inputText = "/unknowncmd"
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
        XCTAssertFalse(vm.acceptSlashSuggestion())
        XCTAssertEqual(vm.inputText, "/unknowncmd")
    }

    func testDiacriticsPreservedInInput() {
        let vm = ChatViewModel()
        let text = "á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž"
        vm.inputText = text
        XCTAssertEqual(vm.inputText, text)
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
    }
}
