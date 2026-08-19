import XCTest
import AppKit
@testable import bagent

@MainActor
final class SlashCommandRegistryTests: XCTestCase {
    func testEmptyAndOrdinaryInputHaveNoSuggestions() {
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "").isEmpty)
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "hello").isEmpty)
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "nájdi faktúru s DPH").isEmpty)
        // Whitespace after the command word means an ordinary prompt.
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "/settings please").isEmpty)
    }

    func testPrefixFilteringIsCaseInsensitive() {
        XCTAssertEqual(SlashCommandRegistry.suggestions(for: "/").map(\.id), ["settings", "automations", "clear"])
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
        XCTAssertNil(SlashCommandRegistry.exactMatch("  /nastavenia "))
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

    func testRawCandidateAndExactMatchingNeverNormalizeInput() {
        XCTAssertNil(SlashCommandRegistry.exactMatch(" /settings"))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/settings "))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/settings\n"))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/auto"))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/path/to/file"))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/https://example.com"))
        XCTAssertNil(SlashCommandRegistry.exactMatch("/automatizacie\u{301}"))
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/AUTOMATIONS")?.id, "automations")
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/automatizacie")?.id, "automations")
        XCTAssertEqual(SlashCommandRegistry.exactMatch("/automatizácie")?.id, "automations")
    }

    func testMarkedTextOwnsCandidateAndExecution() {
        XCTAssertTrue(SlashCommandRegistry.suggestions(for: "/settings", hasMarkedText: true).isEmpty)
        XCTAssertNil(SlashCommandRegistry.exactMatch("/settings", hasMarkedText: true))
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/settings",
                key: .return,
                suggestionsVisible: true,
                hasMarkedText: true
            )),
            .native
        )
    }

    func testCharacterEntryNarrowsWithoutChangingAutomationsDraft() {
        var draft = ""
        for character in "/automations" {
            draft.append(character)
            let suggestions = SlashCommandRegistry.suggestions(for: draft)
            XCTAssertTrue(suggestions.allSatisfy { command in
                command.command.range(
                    of: draft,
                    options: [.anchored, .caseInsensitive]
                ) != nil || command.aliases.contains { alias in
                    alias.range(of: draft, options: [.anchored, .caseInsensitive]) != nil
                }
            })
        }
        XCTAssertEqual(draft, "/automations")
    }

    func testReturnAndCompletionDecisionsAreSeparate() {
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/auto",
                key: .return,
                suggestionsVisible: true
            )),
            .submitOrdinaryPrompt("/auto")
        )
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/auto",
                key: .tab,
                suggestionsVisible: true
            )),
            .completeSuggestion
        )
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/settings",
                key: .return,
                suggestionsVisible: true
            )),
            .execute(SlashCommandRegistry.all[0])
        )
    }

    func testModifiedEnterAndKeyboardPrecedenceStayNative() {
        for key in [SlashCommandInputContext.Key.return, .keypadEnter] {
            for modifier in [
                SlashCommandInputContext.Modifiers.shift,
                .command,
                .option,
                .control,
            ] {
                XCTAssertEqual(
                    SlashCommandInputDecision.resolve(.init(
                        rawDraft: "/settings",
                        key: key,
                        modifiers: modifier,
                        suggestionsVisible: true
                    )),
                    .native
                )
            }
        }
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/s",
                key: .up,
                suggestionsVisible: true
            )),
            .selectPreviousSuggestion
        )
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/s",
                key: .escape,
                suggestionsVisible: true
            )),
            .dismissSuggestions
        )
        XCTAssertEqual(
            SlashCommandInputDecision.resolve(.init(
                rawDraft: "/settings",
                key: .return,
                suggestionsVisible: true,
                approvalActive: true
            )),
            .native
        )
    }

    func testModifiedReturnSuppressionExpiresWithItsEvent() async {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)
        viewModel.inputText = "/settings"
        viewModel.preserveModifiedReturnForNativeEditing()

        let eventEnded = expectation(description: "modified key event ended")
        DispatchQueue.main.async { eventEnded.fulfill() }
        await fulfillment(of: [eventEnded], timeout: 1)

        viewModel.send()
        XCTAssertEqual(viewModel.notchInteractionMode, .settings)
    }

    func testModifiedReturnNeverSubmitsPartialSlashText() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)
        viewModel.inputText = "/auto"
        viewModel.preserveModifiedReturnForNativeEditing()

        viewModel.send()

        XCTAssertEqual(viewModel.inputText, "/auto")
        XCTAssertTrue(viewModel.messages.isEmpty)
        XCTAssertNil(viewModel.slashCommandError)
    }

    func testRegistryOwnsDescriptionDestinationConfirmationAndRouting() {
        let clear = SlashCommandRegistry.exactMatch("/clear")
        XCTAssertEqual(clear?.destination, .currentChat)
        XCTAssertEqual(clear?.confirmationPolicy, .whenCurrentChatIsNonEmpty)
        XCTAssertEqual(clear?.subtitle, "Clear Current Chat")
        XCTAssertEqual(Set(SlashCommandRegistry.all.map(\.id)).count, SlashCommandRegistry.all.count)
    }

    func testTabOrClickCompletionPreservesInputModeAndNeverExecutes() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)
        viewModel.inputText = "/auto"

        XCTAssertTrue(viewModel.completeSlashSuggestion())
        XCTAssertEqual(viewModel.inputText, "/automations")
        XCTAssertEqual(viewModel.notchInteractionMode, .input)
        XCTAssertTrue(viewModel.slashSuggestions.isEmpty)
    }

    func testNativeCompletionPreservesFocusCaretAndUndo() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 80),
            styleMask: [.titled], backing: .buffered, defer: false)
        let editor = NSTextView(frame: window.contentView!.bounds)
        editor.allowsUndo = true
        editor.string = "/auto"
        editor.setSelectedRange(NSRange(location: 2, length: 2))
        window.contentView?.addSubview(editor)
        window.makeFirstResponder(editor)

        XCTAssertTrue(SlashCommandTextCompletion.replaceDraft(in: editor, with: "/automations"))
        XCTAssertTrue(window.firstResponder === editor)
        XCTAssertEqual(editor.string, "/automations")
        XCTAssertEqual(editor.selectedRange(), NSRange(location: 12, length: 0))

        editor.undoManager?.undo()
        XCTAssertEqual(editor.string, "/auto")

        editor.setSelectedRange(NSRange(location: 1, length: 3))
        CurrentChatTextRestoration.placeCaretAtEnd(in: editor)
        XCTAssertEqual(editor.selectedRange(), NSRange(location: 5, length: 0))
    }

    func testPasteOnlyDisplaysSuggestions() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)

        viewModel.inputText = "/settings"

        XCTAssertEqual(viewModel.inputText, "/settings")
        XCTAssertEqual(viewModel.slashSuggestions.map(\.id), ["settings"])
        XCTAssertEqual(viewModel.notchInteractionMode, .input)
    }

    func testMarkedTextHidesSuggestionsWithoutChangingDraft() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.inputText = "/settings"
        viewModel.hasUncommittedMarkedText = true

        XCTAssertEqual(viewModel.inputText, "/settings")
        XCTAssertTrue(viewModel.slashSuggestions.isEmpty)

        viewModel.hasUncommittedMarkedText = false
        XCTAssertEqual(viewModel.slashSuggestions.map(\.id), ["settings"])
    }

    func testCommandFailurePreservesExactDraftAndNeverFallsThrough() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.openInput)
        viewModel.inputText = "/clear"

        viewModel.execute(SlashCommandRegistry.exactMatch("/clear")!)

        XCTAssertEqual(viewModel.inputText, "/clear")
        XCTAssertNotNil(viewModel.slashCommandError)
        XCTAssertTrue(viewModel.messages.isEmpty)
        XCTAssertEqual(viewModel.notchInteractionMode, .input)
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

    func testCompletionUsesCanonicalSpellingWithoutExecution() {
        let vm = ChatViewModel()
        vm.inputText = "/SE"
        XCTAssertTrue(vm.completeSlashSuggestion())
        XCTAssertEqual(vm.inputText, "/settings")
        XCTAssertNotEqual(vm.notchInteractionMode, .settings)
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
    }

    func testUnknownSlashTextStaysEditable() {
        let vm = ChatViewModel()
        vm.inputText = "/unknowncmd"
        XCTAssertTrue(vm.slashSuggestions.isEmpty)
        XCTAssertFalse(vm.completeSlashSuggestion())
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
