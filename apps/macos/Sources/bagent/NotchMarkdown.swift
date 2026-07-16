import AppKit

/// Lightweight markdown → NSAttributedString renderer for the notch output view.
///
/// Produces styled attribute runs only — no containers, boxes, or backgrounds —
/// so it can drop straight into the notch `NSTextView` without altering any
/// layout, width, or panel-height behaviour. The same output feeds both the
/// render path (`attributedText`) and the measurement path
/// (`NotchOutputLayout.textHeight`) so the panel keeps auto-fitting.
///
/// Supported: bold, italic, strikethrough, inline code, links, ATX headers,
/// and unordered/ordered lists. Fenced code blocks are intentionally not
/// handled. Unclosed inline markers render verbatim (live-stream friendly).
enum NotchMarkdown {
    // Base body style — must mirror the plain-text notch style exactly.
    private static let bodySize: CGFloat = 13
    private static let lineSpacing: CGFloat = 1.5
    private static let primaryColor = NotchWrapMetrics.notchTextPrimaryNS      // white 0.80
    private static let codeColor = NSColor(white: 0.66, alpha: 1)              // subtle dimmer tint
    private static let linkColor = NSColor.controlAccentColor
    private static let listIndentStep: CGFloat = 14

    /// Style carried through the inline scanner.
    private struct Inline {
        var size: CGFloat
        var headerBold: Bool
        var bold = false
        var italic = false
        var strike = false
    }

    // Single-entry memo: during streaming the same full text is parsed by the
    // width measure, the height measure, and the render path in one update.
    // Main-thread only (all callers are AppKit/SwiftUI view code).
    nonisolated(unsafe) private static var lastInput: String?
    nonisolated(unsafe) private static var lastOutput: NSAttributedString?

    static func attributedString(_ text: String) -> NSAttributedString {
        if text == lastInput, let lastOutput { return lastOutput }
        let out = NSMutableAttributedString()
        let lines = text.components(separatedBy: "\n")
        for (idx, line) in lines.enumerated() {
            out.append(renderLine(line))
            if idx != lines.count - 1 {
                out.append(NSAttributedString(string: "\n"))
            }
        }
        lastInput = text
        lastOutput = out
        return out
    }

    // MARK: - Block pass

    private static func renderLine(_ line: String) -> NSAttributedString {
        // Header: #{1,6} followed by space.
        if let (level, rest) = headerMatch(line) {
            let size: CGFloat = level <= 1 ? 17 : (level == 2 ? 15 : 13.5)
            let para = baseParagraph()
            para.paragraphSpacingBefore = 2
            var style = Inline(size: size, headerBold: true)
            return parseInline(rest, style: &style, paragraph: para)
        }

        // Unordered list: (spaces)[-*+] + space.
        if let (indentSpaces, rest) = unorderedMatch(line) {
            return listLine(marker: "•  ", content: rest, indentSpaces: indentSpaces)
        }

        // Ordered list: (spaces)\d+. + space.
        if let (indentSpaces, number, rest) = orderedMatch(line) {
            return listLine(marker: "\(number).  ", content: rest, indentSpaces: indentSpaces)
        }

        // Plain paragraph.
        var style = Inline(size: bodySize, headerBold: false)
        return parseInline(line, style: &style, paragraph: baseParagraph())
    }

    private static func listLine(marker: String, content: String, indentSpaces: Int) -> NSAttributedString {
        let depth = CGFloat(indentSpaces / 2)
        let base = depth * listIndentStep + listIndentStep
        let para = baseParagraph()
        para.firstLineHeadIndent = base
        para.headIndent = base + listIndentStep
        let result = NSMutableAttributedString()
        let markerStyle = Inline(size: bodySize, headerBold: false)
        result.append(NSAttributedString(string: marker, attributes: attributes(markerStyle, paragraph: para)))
        var style = Inline(size: bodySize, headerBold: false)
        result.append(parseInline(content, style: &style, paragraph: para))
        return result
    }

    // MARK: - Inline pass

    private static func parseInline(_ text: String, style: inout Inline, paragraph: NSParagraphStyle) -> NSAttributedString {
        let result = NSMutableAttributedString()
        let chars = Array(text)
        var i = 0
        var literal = ""

        func flushLiteral() {
            guard !literal.isEmpty else { return }
            result.append(NSAttributedString(string: literal, attributes: attributes(style, paragraph: paragraph)))
            literal = ""
        }

        while i < chars.count {
            // Inline code `...` — literal, no nested styling.
            if chars[i] == "`", let close = findClose(chars, from: i + 1, char: "`") {
                flushLiteral()
                let inner = String(chars[(i + 1)..<close])
                let codeStyle = style
                result.append(NSAttributedString(string: inner, attributes: codeAttributes(codeStyle, paragraph: paragraph)))
                i = close + 1
                continue
            }
            // Link [label](url)
            if chars[i] == "[", let link = parseLink(chars, from: i) {
                flushLiteral()
                var labelStyle = style
                let label = parseInline(link.label, style: &labelStyle, paragraph: paragraph)
                let mutable = NSMutableAttributedString(attributedString: label)
                mutable.addAttributes(
                    [.foregroundColor: linkColor,
                     .underlineStyle: NSUnderlineStyle.single.rawValue,
                     .link: link.url],
                    range: NSRange(location: 0, length: mutable.length))
                result.append(mutable)
                i = link.end
                continue
            }
            // Strikethrough ~~...~~
            if matchesSeq(chars, i, "~~"), let close = findCloseSeq(chars, from: i + 2, seq: ["~", "~"]) {
                flushLiteral()
                var inner = style; inner.strike = true
                result.append(parseInline(String(chars[(i + 2)..<close]), style: &inner, paragraph: paragraph))
                i = close + 2
                continue
            }
            // Bold ** or __
            if let seq = boldSeq(chars, i), let close = findCloseSeq(chars, from: i + 2, seq: seq) {
                flushLiteral()
                var inner = style; inner.bold = true
                result.append(parseInline(String(chars[(i + 2)..<close]), style: &inner, paragraph: paragraph))
                i = close + 2
                continue
            }
            // Italic * or _
            if let c = italicChar(chars, i), let close = findClose(chars, from: i + 1, char: c) {
                flushLiteral()
                var inner = style; inner.italic = true
                result.append(parseInline(String(chars[(i + 1)..<close]), style: &inner, paragraph: paragraph))
                i = close + 1
                continue
            }
            literal.append(chars[i])
            i += 1
        }
        flushLiteral()
        return result
    }

    // MARK: - Attribute builders

    private static func attributes(_ s: Inline, paragraph: NSParagraphStyle) -> [NSAttributedString.Key: Any] {
        var attrs: [NSAttributedString.Key: Any] = [
            .font: makeFont(size: s.size, bold: s.bold || s.headerBold, italic: s.italic),
            .foregroundColor: primaryColor,
            .paragraphStyle: paragraph,
        ]
        if s.strike { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
        return attrs
    }

    private static func codeAttributes(_ s: Inline, paragraph: NSParagraphStyle) -> [NSAttributedString.Key: Any] {
        var attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: codeColor,
            .paragraphStyle: paragraph,
        ]
        if s.strike { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
        return attrs
    }

    private static func makeFont(size: CGFloat, bold: Bool, italic: Bool) -> NSFont {
        let base = NSFont.systemFont(ofSize: size, weight: bold ? .semibold : .regular)
        var traits: NSFontDescriptor.SymbolicTraits = []
        if bold { traits.insert(.bold) }
        if italic { traits.insert(.italic) }
        guard !traits.isEmpty else { return base }
        let desc = base.fontDescriptor.withSymbolicTraits(traits)
        return NSFont(descriptor: desc, size: size) ?? base
    }

    private static func baseParagraph() -> NSMutableParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineSpacing = lineSpacing
        p.lineBreakMode = .byWordWrapping
        return p
    }

    // MARK: - Block matchers

    private static func headerMatch(_ line: String) -> (Int, String)? {
        var count = 0
        let chars = Array(line)
        while count < chars.count && chars[count] == "#" { count += 1 }
        guard count >= 1, count <= 6, count < chars.count, chars[count] == " " else { return nil }
        let rest = String(chars[(count + 1)...]).drop { $0 == " " }
        return (count, String(rest))
    }

    private static func leadingSpaces(_ line: String) -> Int {
        var n = 0
        for c in line { if c == " " { n += 1 } else { break } }
        return n
    }

    private static func unorderedMatch(_ line: String) -> (Int, String)? {
        let spaces = leadingSpaces(line)
        let chars = Array(line)
        guard spaces < chars.count else { return nil }
        let marker = chars[spaces]
        guard marker == "-" || marker == "*" || marker == "+" else { return nil }
        guard spaces + 1 < chars.count, chars[spaces + 1] == " " else { return nil }
        let rest = String(chars[(spaces + 2)...]).drop { $0 == " " }
        return (spaces, String(rest))
    }

    private static func orderedMatch(_ line: String) -> (Int, Int, String)? {
        let spaces = leadingSpaces(line)
        let chars = Array(line)
        var i = spaces
        var digits = ""
        while i < chars.count, chars[i].isNumber { digits.append(chars[i]); i += 1 }
        guard !digits.isEmpty, let number = Int(digits) else { return nil }
        guard i < chars.count, chars[i] == ".", i + 1 < chars.count, chars[i + 1] == " " else { return nil }
        let rest = String(chars[(i + 2)...]).drop { $0 == " " }
        return (spaces, number, String(rest))
    }

    // MARK: - Inline scanners

    private static func findClose(_ chars: [Character], from: Int, char: Character) -> Int? {
        var i = from
        while i < chars.count {
            if chars[i] == char { return i }
            i += 1
        }
        return nil
    }

    private static func findCloseSeq(_ chars: [Character], from: Int, seq: [Character]) -> Int? {
        var i = from
        while i + seq.count <= chars.count {
            if Array(chars[i..<(i + seq.count)]) == seq { return i }
            i += 1
        }
        return nil
    }

    private static func matchesSeq(_ chars: [Character], _ i: Int, _ seq: String) -> Bool {
        let s = Array(seq)
        guard i + s.count <= chars.count else { return false }
        return Array(chars[i..<(i + s.count)]) == s
    }

    private static func boldSeq(_ chars: [Character], _ i: Int) -> [Character]? {
        if matchesSeq(chars, i, "**") { return ["*", "*"] }
        if matchesSeq(chars, i, "__") { return ["_", "_"] }
        return nil
    }

    private static func italicChar(_ chars: [Character], _ i: Int) -> Character? {
        let c = chars[i]
        guard c == "*" || c == "_" else { return nil }
        return c
    }

    private static func parseLink(_ chars: [Character], from: Int) -> (label: String, url: String, end: Int)? {
        // chars[from] == "["
        guard let labelEnd = findClose(chars, from: from + 1, char: "]") else { return nil }
        guard labelEnd + 1 < chars.count, chars[labelEnd + 1] == "(" else { return nil }
        guard let urlEnd = findClose(chars, from: labelEnd + 2, char: ")") else { return nil }
        let label = String(chars[(from + 1)..<labelEnd])
        let url = String(chars[(labelEnd + 2)..<urlEnd])
        guard !url.isEmpty else { return nil }
        return (label, url, urlEnd + 1)
    }
}
