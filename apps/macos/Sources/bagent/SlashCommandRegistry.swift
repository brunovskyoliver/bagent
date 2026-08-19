import Foundation
import AppKit

/// A typed notch slash command. Adding a command means adding one entry to
/// `SlashCommandRegistry.all` — matching, suggestions, and execution routing
/// all derive from it.
struct SlashCommand: Identifiable, Equatable {
    enum Destination: Equatable {
        case settings
        case automations
        case currentChat
    }

    enum ConfirmationPolicy: Equatable {
        case none
        case whenCurrentChatIsNonEmpty
    }

    let id: String
    /// Canonical lowercase command text, e.g. "/settings".
    let command: String
    /// Accepted alternative spellings (also lowercase), e.g. "/nastavenia".
    let aliases: [String]
    /// Very short description for the suggestion row.
    let subtitle: String
    /// Optional SF Symbol name.
    let symbol: String?
    let destination: Destination
    let confirmationPolicy: ConfirmationPolicy
}

extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

enum SlashCommandRegistry {
    static let maxSuggestions = 3

    static let all: [SlashCommand] = [
        SlashCommand(
            id: "settings",
            command: "/settings",
            aliases: ["/nastavenia"],
            subtitle: "Open bagent settings",
            symbol: "gearshape",
            destination: .settings,
            confirmationPolicy: .none
        ),
        SlashCommand(
            id: "automations",
            command: "/automations",
            aliases: ["/automatizacie", "/automatizácie"],
            subtitle: "Scheduled agent tasks",
            symbol: "clock.arrow.2.circlepath",
            destination: .automations,
            confirmationPolicy: .none
        ),
        SlashCommand(
            id: "clear",
            command: "/clear",
            aliases: [],
            subtitle: String(localized: "slashCommand.clear.subtitle", defaultValue: "Clear Current Chat"),
            symbol: "trash",
            destination: .currentChat,
            confirmationPolicy: .whenCurrentChatIsNonEmpty
        ),
    ]

    /// Case-insensitive prefix suggestions for the current input text.
    /// Empty input and non-slash input produce no suggestions; text with
    /// whitespace after the command word is an ordinary prompt.
    static func suggestions(for input: String, hasMarkedText: Bool = false) -> [SlashCommand] {
        guard isCandidate(input, hasMarkedText: hasMarkedText) else { return [] }
        return all
            .filter { cmd in
                matchesPrefix(input, candidate: cmd.command)
                    || cmd.aliases.contains { matchesPrefix(input, candidate: $0) }
            }
            .prefix(maxSuggestions)
            .map { $0 }
    }

    /// The command a complete input (canonical or alias, any case) resolves to.
    static func exactMatch(_ input: String, hasMarkedText: Bool = false) -> SlashCommand? {
        guard isCandidate(input, hasMarkedText: hasMarkedText) else { return nil }
        return all.first { command in
            equalsCaseInsensitively(input, command.command)
                || command.aliases.contains { equalsCaseInsensitively(input, $0) }
        }
    }

    private static func isCandidate(_ input: String, hasMarkedText: Bool) -> Bool {
        !hasMarkedText
            && input.first == "/"
            && !input.contains(where: { $0.isWhitespace })
    }

    private static func equalsCaseInsensitively(_ lhs: String, _ rhs: String) -> Bool {
        lhs.compare(rhs, options: [.caseInsensitive], locale: Locale(identifier: "en_US_POSIX"))
            == .orderedSame
    }

    private static func matchesPrefix(_ prefix: String, candidate: String) -> Bool {
        candidate.range(
            of: prefix,
            options: [.anchored, .caseInsensitive],
            locale: Locale(identifier: "en_US_POSIX")
        ) != nil
    }
}

struct SlashCommandInputContext: Equatable {
    enum Key: Equatable {
        case up
        case down
        case tab
        case escape
        case `return`
        case keypadEnter
    }

    struct Modifiers: OptionSet, Equatable {
        let rawValue: Int

        static let shift = Modifiers(rawValue: 1 << 0)
        static let command = Modifiers(rawValue: 1 << 1)
        static let option = Modifiers(rawValue: 1 << 2)
        static let control = Modifiers(rawValue: 1 << 3)
    }

    let rawDraft: String
    let key: Key
    var modifiers: Modifiers = []
    var suggestionsVisible = false
    var hasMarkedText = false
    var inputOwnsFocus = true
    var conversationTurnActive = false
    var approvalActive = false
}

enum SlashCommandInputDecision: Equatable {
    case native
    case selectPreviousSuggestion
    case selectNextSuggestion
    case completeSuggestion
    case dismissSuggestions
    case execute(SlashCommand)
    case submitOrdinaryPrompt(String)

    static func resolve(_ context: SlashCommandInputContext) -> SlashCommandInputDecision {
        guard context.inputOwnsFocus,
              !context.conversationTurnActive,
              !context.approvalActive,
              !context.hasMarkedText
        else { return .native }

        if context.suggestionsVisible {
            switch context.key {
            case .up where context.modifiers.isEmpty:
                return .selectPreviousSuggestion
            case .down where context.modifiers.isEmpty:
                return .selectNextSuggestion
            case .tab where context.modifiers.isEmpty:
                return .completeSuggestion
            case .escape where context.modifiers.isEmpty:
                return .dismissSuggestions
            default:
                break
            }
        }

        guard context.key == .return || context.key == .keypadEnter else { return .native }
        guard context.modifiers.isEmpty else { return .native }
        if let command = SlashCommandRegistry.exactMatch(context.rawDraft) {
            return .execute(command)
        }
        return .submitOrdinaryPrompt(context.rawDraft)
    }
}

@MainActor
enum SlashCommandTextCompletion {
    @discardableResult
    static func replaceDraft(in editor: NSTextView, with command: String) -> Bool {
        guard !editor.hasMarkedText() else { return false }
        let fullDraft = NSRange(location: 0, length: (editor.string as NSString).length)
        editor.setSelectedRange(fullDraft)
        editor.insertText(command, replacementRange: fullDraft)
        editor.setSelectedRange(NSRange(location: (command as NSString).length, length: 0))
        return true
    }
}

@MainActor
enum CurrentChatTextRestoration {
    static func placeCaretAtEnd(in editor: NSTextView) {
        editor.setSelectedRange(NSRange(location: (editor.string as NSString).length, length: 0))
    }
}
