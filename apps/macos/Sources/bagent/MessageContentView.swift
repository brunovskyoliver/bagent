import SwiftUI
import WebKit

// MARK: - Decides between plain Text and rich WebView

struct MessageContentView: View {
    let text: String
    let isStreaming: Bool

    var body: some View {
        if isStreaming || !needsRichRender(text) {
            Text(text)
                .font(.system(size: 13))
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            WebMessageView(content: text)
        }
    }

    /// Trigger rich rendering if the text has LaTeX, code blocks, or bold markdown.
    private func needsRichRender(_ s: String) -> Bool {
        s.contains("$$")
        || s.contains("\\[")
        || s.contains("\\begin{")
        || s.contains("```")
        || s.contains("**")
        || s.contains("`")
        || (s.contains("$") && s.range(of: #"\$[^$\n]+\$"#, options: .regularExpression) != nil)
    }
}

// MARK: - SwiftUI wrapper around WKWebView with dynamic height

struct WebMessageView: View {
    let content: String
    @State private var height: CGFloat = 60

    var body: some View {
        _WebViewRepresentable(html: buildHTML(content), height: $height)
            .frame(height: max(height, 20))
    }

    // MARK: - HTML generation

    private func buildHTML(_ text: String) -> String {
        let body = processContent(text)
        return """
        <!DOCTYPE html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width,initial-scale=1">
        <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.css">
        <script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.js"></script>
        <script defer src="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/contrib/auto-render.min.js"
            onload="renderMathInElement(document.body,{delimiters:[{left:'$$',right:'$$',display:true},{left:'$',right:'$',display:false},{left:'\\\\[',right:'\\\\]',display:true},{left:'\\\\(',right:'\\\\)',display:false}],throwOnError:false});"></script>
        <style>
        *{box-sizing:border-box}
        html,body{margin:0;padding:0;background:transparent}
        body{font-family:-apple-system,BlinkMacSystemFont;font-size:13px;line-height:1.55;color:CanvasText;color-scheme:light dark;word-wrap:break-word}
        p{margin:0 0 6px}p:last-child{margin-bottom:0}
        code{font-family:'SF Mono',Menlo,monospace;font-size:11.5px;background:rgba(128,128,128,.15);padding:1px 4px;border-radius:3px}
        pre{background:rgba(128,128,128,.12);padding:8px 10px;border-radius:6px;overflow-x:auto;margin:6px 0}
        pre code{background:none;padding:0;font-size:12px}
        strong{font-weight:600}
        .katex-display{overflow-x:auto;overflow-y:hidden;padding:4px 0}
        </style>
        </head>
        <body>\(body)</body>
        </html>
        """
    }

    private func processContent(_ text: String) -> String {
        // 1. HTML-escape (LaTeX delimiters $, \, {, } are not HTML-special — safe)
        var html = text
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")

        // 2. Fenced code blocks
        html = html.replacingOccurrences(
            of: "```(?:[a-z]*\\n)?([\\s\\S]*?)```",
            with: "<pre><code>$1</code></pre>",
            options: .regularExpression
        )

        // 3. Bold (**text**)
        html = html.replacingOccurrences(
            of: "\\*\\*(.+?)\\*\\*",
            with: "<strong>$1</strong>",
            options: .regularExpression
        )

        // 4. Inline code (`text`)
        html = html.replacingOccurrences(
            of: "`([^`]+)`",
            with: "<code>$1</code>",
            options: .regularExpression
        )

        // 5. Paragraphs (double newline) and line breaks
        html = html.components(separatedBy: "\n\n").map { p in
            "<p>\(p.replacingOccurrences(of: "\n", with: "<br>"))</p>"
        }.joined(separator: "\n")

        return html
    }
}

// MARK: - NSViewRepresentable

private struct _WebViewRepresentable: NSViewRepresentable {
    let html: String
    @Binding var height: CGFloat

    func makeNSView(context: Context) -> WKWebView {
        let wv = WKWebView()
        wv.navigationDelegate = context.coordinator
        wv.setValue(false, forKey: "drawsBackground")
        return wv
    }

    func updateNSView(_ wv: WKWebView, context: Context) {
        guard context.coordinator.lastHTML != html else { return }
        context.coordinator.lastHTML = html
        context.coordinator.heightBinding = $height
        wv.loadHTMLString(html, baseURL: nil)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        var lastHTML: String = ""
        var heightBinding: Binding<CGFloat>?

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            // Wait briefly for KaTeX to render before measuring height
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                webView.evaluateJavaScript("document.documentElement.scrollHeight") { [weak self] result, _ in
                    if let h = result as? CGFloat, h > 10 {
                        self?.heightBinding?.wrappedValue = h
                    }
                }
            }
        }

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void
        ) {
            decisionHandler(navigationAction.navigationType == .other ? .allow : .cancel)
        }
    }
}
