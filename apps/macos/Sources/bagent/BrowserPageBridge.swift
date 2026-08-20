import AppKit
import Foundation
import WebKit

struct BrowserConsoleMessage: Codable, Equatable, Sendable {
    let level: String
    let message: String
}

struct BrowserNetworkRequest: Codable, Equatable, Sendable {
    let url: String
    let method: String
    let status: Int?
    let resourceType: String
    let durationMilliseconds: Int?
    let failureReason: String?
}

private struct BrowserDOMActionResponse: Codable {
    let ok: Bool
    let passwordField: Bool
    let nativeRequired: Bool
    let submissionRequired: Bool
    let navigationBegan: Bool
    let error: String?
}

@MainActor
final class BrowserPageBridge: NSObject, WKScriptMessageHandler {
    private(set) var consoleMessages: [BrowserConsoleMessage] = []
    private(set) var networkRequests: [BrowserNetworkRequest] = []
    private(set) var referencesRevision: Int = 0

    private weak var webView: WKWebView?
    private let userContentController: WKUserContentController

    init(webView: WKWebView) {
        self.webView = webView
        self.userContentController = webView.configuration.userContentController
        super.init()
        userContentController.add(self, name: "bagentBrowserEvent")
        let script = WKUserScript(
            source: Self.instrumentationScript,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        userContentController.addUserScript(script)
    }

    func invalidateReferences(revision: Int) {
        referencesRevision = revision
    }

    func snapshot(revision: Int, maxCharacters: Int = 20_000) async throws -> BrowserPageSnapshot {
        guard let webView else { throw BrowserFailure(.browserProcessTerminated, "The WebKit content process is unavailable.") }
        referencesRevision = revision
        let script = "JSON.stringify(\(Self.snapshotFunction)(\(revision), \(max(1_000, min(maxCharacters, 40_000)))))"
        let json = try await evaluateJSONString(script, in: webView)
        do {
            return try JSONDecoder().decode(BrowserPageSnapshot.self, from: Data(json.utf8))
        } catch {
            throw BrowserFailure(.operationTimedOut, "The page snapshot could not be decoded.")
        }
    }

    func perform(_ action: BrowserAction, allowSubmission: Bool) async throws -> BrowserInteractionEvaluation {
        guard let webView else { throw BrowserFailure(.browserProcessTerminated, "The WebKit content process is unavailable.") }
        guard let reference = action.reference else {
            if action.x != nil, action.y != nil {
                throw BrowserFailure(.visibleInteractionRequired, "Coordinate interaction requires the visible Browser Panel in v1.")
            }
            throw BrowserFailure(.invalidRequest, "A semantic action requires an Element Reference.")
        }
        guard referencesRevision == action.revision else {
            throw BrowserFailure(.staleElementReference, "The Element Reference belongs to an older Page Snapshot revision.")
        }

        let referenceJSON = try jsonString(reference)
        let textJSON = try jsonString(String((action.text ?? "").prefix(8_000)))
        let keyJSON = try jsonString(String((action.key ?? "").prefix(100)))
        let typeJSON = try jsonString(action.type)
        let deltaXJSON = try jsonNumber(action.deltaX)
        let deltaYJSON = try jsonNumber(action.deltaY)
        let script = "JSON.stringify(\(Self.actionFunction)(\(referenceJSON), \(typeJSON), \(textJSON), \(keyJSON), \(deltaXJSON), \(deltaYJSON), \(allowSubmission ? "true" : "false")))"
        let json = try await evaluateJSONString(script, in: webView)
        let result = try JSONDecoder().decode(BrowserDOMActionResponse.self, from: Data(json.utf8))
        return BrowserInteractionEvaluation(
            ok: result.ok,
            passwordField: result.passwordField,
            nativeRequired: result.nativeRequired,
            submissionRequired: result.submissionRequired,
            navigationBegan: result.navigationBegan,
            error: result.error
        )
    }

    func acknowledgeAlert() -> Bool {
        false
    }

    private func jsonString(_ value: String) throws -> String {
        let data = try JSONEncoder().encode(value)
        return String(decoding: data, as: UTF8.self)
    }

    private func jsonNumber(_ value: Double?) throws -> String {
        guard let value else { return "null" }
        let data = try JSONEncoder().encode(value)
        return String(decoding: data, as: UTF8.self)
    }

    private func evaluateJSONString(_ script: String, in webView: WKWebView) async throws -> String {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<String, Error>) in
            webView.evaluateJavaScript(script) { value, error in
                if let error {
                    let nsError = error as NSError
                    continuation.resume(throwing: BrowserFailure(
                        .invalidRequest,
                        "WebKit could not complete the semantic browser operation (\(nsError.domain)/\(nsError.code))."
                    ))
                } else if let value = value as? String {
                    continuation.resume(returning: value)
                } else {
                    continuation.resume(throwing: BrowserFailure(.operationTimedOut, "The page did not return a structured result."))
                }
            }
        }
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any], let kind = body["kind"] as? String else { return }
        switch kind {
        case "console":
            let level = String((body["level"] as? String ?? "log").prefix(32))
            consoleMessages.append(BrowserConsoleMessage(level: level, message: "[page console message]"))
            if consoleMessages.count > 100 { consoleMessages.removeFirst(consoleMessages.count - 100) }
        case "network":
            guard let url = body["url"] as? String else { return }
            networkRequests.append(BrowserNetworkRequest(
                url: Self.redactURL(url),
                method: String((body["method"] as? String ?? "GET").prefix(16)),
                status: body["status"] as? Int,
                resourceType: String((body["resourceType"] as? String ?? "fetch").prefix(32)),
                durationMilliseconds: body["durationMilliseconds"] as? Int,
                failureReason: (body["failureReason"] as? String).map { _ in "request_failed" }
            ))
            if networkRequests.count > 200 { networkRequests.removeFirst(networkRequests.count - 200) }
        default:
            break
        }
    }

    private static func redactURL(_ raw: String) -> String {
        guard var components = URLComponents(string: raw) else { return "[invalid-url]" }
        components.query = nil
        components.fragment = nil
        return components.string ?? "[invalid-url]"
    }

    private static let instrumentationScript = """
    (() => {
      if (window.__bagentBrowserInstrumented) return;
      window.__bagentBrowserInstrumented = true;
      const post = (payload) => {
        try { window.webkit.messageHandlers.bagentBrowserEvent.postMessage(payload); } catch (_) {}
      };
      for (const level of ['log','info','warn','error','debug']) {
        const original = console[level];
        console[level] = (...args) => {
          post({kind:'console', level});
          original.apply(console, args);
        };
      }
      const sendNetwork = (url, method, status, resourceType, started, failureReason) => post({
        kind:'network', url:String(url), method:String(method || 'GET'), status:status == null ? null : Number(status),
        resourceType, durationMilliseconds:Math.round(performance.now() - started), failureReason:failureReason || null
      });
      const originalFetch = window.fetch;
      window.fetch = (...args) => {
        const started = performance.now(); const request = args[0];
        const url = request && request.url ? request.url : String(request);
        const method = args[1] && args[1].method ? args[1].method : (request && request.method ? request.method : 'GET');
        return originalFetch(...args).then(response => { sendNetwork(url, method, response.status, 'fetch', started, null); return response; }, error => { sendNetwork(url, method, null, 'fetch', started, String(error)); throw error; });
      };
      const originalOpen = XMLHttpRequest.prototype.open; const originalSend = XMLHttpRequest.prototype.send;
      XMLHttpRequest.prototype.open = function(method, url) { this.__bagentMethod = method; this.__bagentURL = url; return originalOpen.apply(this, arguments); };
      XMLHttpRequest.prototype.send = function() { const xhr = this; const started = performance.now(); const finish = () => sendNetwork(xhr.__bagentURL || '', xhr.__bagentMethod || 'GET', xhr.status || null, 'xmlhttprequest', started, xhr.status ? null : 'request_failed'); xhr.addEventListener('loadend', finish, {once:true}); return originalSend.apply(this, arguments); };
      const denied = (name) => (...args) => Promise.reject(new DOMException(name, 'NotAllowedError'));
      try { if (navigator.mediaDevices) navigator.mediaDevices.getUserMedia = denied('permission_not_supported'); } catch (_) {}
      try { if (navigator.geolocation) { navigator.geolocation.getCurrentPosition = (_success, error) => { if (error) error({code:1,message:'permission_not_supported'}); }; navigator.geolocation.watchPosition = (_success, error) => { if (error) error({code:1,message:'permission_not_supported'}); return -1; }; } } catch (_) {}
      try { if (navigator.clipboard) { navigator.clipboard.readText = denied('permission_not_supported'); navigator.clipboard.writeText = denied('permission_not_supported'); } } catch (_) {}
      try { if (window.Notification) window.Notification.requestPermission = () => Promise.resolve('denied'); } catch (_) {}
      try { if (navigator.credentials) { navigator.credentials.get = denied('permission_not_supported'); navigator.credentials.create = denied('permission_not_supported'); } } catch (_) {}
      try { if (window.showOpenFilePicker) window.showOpenFilePicker = denied('visible_interaction_required'); if (window.showSaveFilePicker) window.showSaveFilePicker = denied('permission_not_supported'); } catch (_) {}
      try { window.open = () => null; } catch (_) {}
    })();
    """

    private static let snapshotFunction = """
    (function(revision, maxCharacters) {
      const trim = (value, limit=400) => String(value || '').replace(/\\s+/g, ' ').trim().slice(0, limit);
      const visible = (node) => { const r = node.getBoundingClientRect(); const s = getComputedStyle(node); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
      const rect = (node) => { const r = node.getBoundingClientRect(); return {x:r.x, y:r.y, width:r.width, height:r.height}; };
      const role = (node) => node.getAttribute('role') || ({A:'link',BUTTON:'button',INPUT:(node.type === 'checkbox' ? 'checkbox' : node.type === 'radio' ? 'radio' : node.type === 'submit' ? 'button' : 'textbox'),TEXTAREA:'textbox',SELECT:'combobox'}[node.tagName] || (node.isContentEditable ? 'textbox' : 'generic'));
      const name = (node) => trim(node.getAttribute('aria-label') || node.getAttribute('name') || node.innerText || node.textContent, 200);
      const element = (node, ref) => ({reference:ref, role:role(node), accessibleName:name(node), state:{disabled:String(!!node.disabled), checked:String(!!node.checked), expanded:String(node.getAttribute('aria-expanded') || '')}, bounds:rect(node), sensitive:node.matches('input[type="password"]'), submitsForm:!!node.closest('form') && node.matches('button[type="submit"],input[type="submit"],button:not([type])')});
      let next = 0; window.__bagentRefs = Object.create(null);
      const elements = Array.from(document.querySelectorAll('a,button,input,textarea,select,[contenteditable="true"],[role]')).filter(visible).slice(0, 300).map(node => { const ref = 'e_' + revision + '_' + (++next); window.__bagentRefs[ref] = node; return element(node, ref); });
      const frames = Array.from(document.querySelectorAll('iframe,frame')).slice(0, 20).map(frame => {
        let origin = 'opaque'; let nested = []; let opaque = true;
        try { origin = frame.contentWindow.location.origin; const doc = frame.contentDocument; if (doc) { opaque = false; nested = Array.from(doc.querySelectorAll('a,button,input,textarea,select,[contenteditable="true"],[role]')).filter(visible).slice(0, 100).map(node => { const ref = 'frame_' + revision + '_' + (++next); window.__bagentRefs[ref] = node; return element(node, ref); }); } } catch (_) {}
        return {origin, bounds:rect(frame), opaque, elements:nested};
      });
      const visibleText = trim(document.body && document.body.innerText, maxCharacters);
      return {revision, url:location.href, title:document.title, visibleText, headings:Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6')).filter(visible).slice(0,50).map(n=>trim(n.innerText,200)), landmarks:Array.from(document.querySelectorAll('main,nav,header,footer,aside,[role="main"],[role="navigation"],[role="region"]')).filter(visible).slice(0,50).map(n=>n.getAttribute('role') || n.tagName.toLowerCase()), forms:Array.from(document.querySelectorAll('form')).slice(0,50).map(n=>trim(n.getAttribute('aria-label') || n.getAttribute('name') || 'form',100)), elements, frames, viewport:{width:innerWidth,height:innerHeight,backingScale:devicePixelRatio}, bounded:true};
    })
    """

    private static let actionFunction = """
    (function(ref, type, value, key, deltaX, deltaY, allowSubmission) {
      const node = window.__bagentRefs && window.__bagentRefs[ref];
      if (!node || !document.contains(node) && !node.getRootNode) return {ok:false,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:'missing_reference'};
      const passwordField = node.matches('input[type="password"]');
      if (passwordField) return {ok:false,passwordField:true,nativeRequired:true,submissionRequired:false,navigationBegan:false,error:'password_field'};
      if (node.matches('input[type="file"]')) return {ok:false,passwordField:false,nativeRequired:true,submissionRequired:false,navigationBegan:false,error:'native_input'};
      const submissionRequired = !!node.closest('form') && node.matches('button[type="submit"],input[type="submit"],button:not([type])');
      if (submissionRequired && !allowSubmission) return {ok:false,passwordField:false,nativeRequired:false,submissionRequired:true,navigationBegan:false,error:'submission_grant'};
      if (type === 'click') { node.click(); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:submissionRequired,error:null}; }
      if (type === 'focus') { node.focus({preventScroll:true}); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:null}; }
      if (type === 'type') { if (node.isContentEditable) node.textContent = value; else node.value = value; node.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value})); node.dispatchEvent(new Event('change',{bubbles:true})); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:null}; }
      if (type === 'press') { node.dispatchEvent(new KeyboardEvent('keydown',{key,code:key,bubbles:true})); node.dispatchEvent(new KeyboardEvent('keyup',{key,code:key,bubbles:true})); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:null}; }
      if (type === 'hover' || type === 'move') { node.dispatchEvent(new MouseEvent('mouseover',{bubbles:true,clientX:node.getBoundingClientRect().x,clientY:node.getBoundingClientRect().y})); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:null}; }
      if (type === 'scroll') { node.scrollBy({top:Number(deltaY)||0,left:Number(deltaX)||0,behavior:'instant'}); return {ok:true,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:null}; }
      return {ok:false,passwordField:false,nativeRequired:false,submissionRequired:false,navigationBegan:false,error:'unsupported_action'};
    })
    """
}

struct BrowserInteractionEvaluation: Sendable {
    let ok: Bool
    let passwordField: Bool
    let nativeRequired: Bool
    let submissionRequired: Bool
    let navigationBegan: Bool
    let error: String?
}
