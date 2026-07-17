import Foundation

/// A typed notch slash command. Adding a command means adding one entry to
/// `SlashCommandRegistry.all` — matching, suggestions, and execution routing
/// all derive from it.
struct SlashCommand: Identifiable, Equatable {
    enum Action: Equatable {
        case openSettings
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
    let action: Action
}

extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

enum SlashCommandRegistry {
    static let maxSuggestions = 3

    /// /automations is registered by the automations surface issue — do not
    /// add it before that surface exists.
    static let all: [SlashCommand] = [
        SlashCommand(
            id: "settings",
            command: "/settings",
            aliases: ["/nastavenia"],
            subtitle: "Open bagent settings",
            symbol: "gearshape",
            action: .openSettings
        ),
    ]

    /// Case-insensitive prefix suggestions for the current input text.
    /// Empty input and non-slash input produce no suggestions; text with
    /// whitespace after the command word is an ordinary prompt.
    static func suggestions(for input: String) -> [SlashCommand] {
        let text = input.lowercased()
        guard text.hasPrefix("/"), !text.contains(where: { $0.isWhitespace }) else { return [] }
        return all
            .filter { cmd in
                cmd.command.hasPrefix(text) || cmd.aliases.contains { $0.hasPrefix(text) }
            }
            .prefix(maxSuggestions)
            .map { $0 }
    }

    /// The command a complete input (canonical or alias, any case) resolves to.
    static func exactMatch(_ input: String) -> SlashCommand? {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return all.first { $0.command == text || $0.aliases.contains(text) }
    }
}
