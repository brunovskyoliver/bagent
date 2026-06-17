import AppKit
import ApplicationServices
import CoreGraphics
import ScreenCaptureKit
import Vision

/// On-demand screen context capture (Phase 7).
///
/// Produces an ephemeral `ScreenContext` struct from:
///   - A PNG screenshot (ScreenCaptureKit, fallback CGWindowListCreateImage)
///   - On-device OCR of the captured frame (Vision VNRecognizeTextRequest)
///   - Frontmost application name via NSWorkspace (no extra permission needed)
///   - Accessibility selected-text (password-field safe-guarded)
///
/// **No image bytes are ever written to disk.** The base64 PNG lives only in
/// memory for the lifetime of the `/chat` request.

struct ScreenContext {
    /// Base64-encoded PNG of the main display (nil if capture failed / not authorized).
    var imagePNGBase64: String?
    /// Joined text recognised by Vision OCR (empty when no image or no text found).
    var ocrText: String
    /// Frontmost app as "AppName (bundle.id)" or just "AppName".
    var activeApp: String?
    /// Accessibility selected text; nil when nothing selected, AX denied, or password field.
    var selectedText: String?
}

@MainActor
final class ScreenContextProvider {

    static let shared = ScreenContextProvider()
    private init() {}

    // MARK: - Max image dimension to keep the vision payload manageable

    private let maxLongEdge: CGFloat = 1568

    // MARK: - Public capture entry point

    /// Capture screen context as requested.
    ///
    /// - Parameters:
    ///   - wantsScreen:    Capture a screenshot and run OCR.
    ///   - wantsOCR:       Run on-device OCR (only meaningful when `wantsScreen` is true).
    ///   - wantsSelection: Read the Accessibility selected-text instead of / in addition
    ///                     to the screenshot.
    func capture(wantsScreen: Bool, wantsOCR: Bool, wantsSelection: Bool) async -> ScreenContext {
        async let appName     = frontmostApp()
        async let selection   = wantsSelection ? selectedText() : nil

        var imagePNGBase64: String?
        var ocrText = ""

        if wantsScreen {
            if let cgImage = await captureFrame() {
                // PNG encode in-memory only — never write to disk
                imagePNGBase64 = pngBase64(cgImage)
                if wantsOCR {
                    ocrText = await recognizeText(in: cgImage)
                }
            }
        }

        return ScreenContext(
            imagePNGBase64: imagePNGBase64,
            ocrText: ocrText,
            activeApp: await appName,
            selectedText: await selection
        )
    }

    // MARK: - Screenshot

    // ScreenCaptureKit is available on all supported macOS versions (14.0+).
    private func captureFrame() async -> CGImage? {
        do {
            let content = try await SCShareableContent.current
            guard let display = content.displays.first else { return nil }

            let filter = SCContentFilter(display: display, excludingWindows: [])
            let config = SCStreamConfiguration()
            config.width  = Int(display.width)
            config.height = Int(display.height)
            config.showsCursor = false

            let image = try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: config
            )
            return downscale(image)
        } catch {
            return nil
        }
    }

    // MARK: - Downscale to fit within maxLongEdge

    private func downscale(_ source: CGImage) -> CGImage {
        let w = CGFloat(source.width)
        let h = CGFloat(source.height)
        let longEdge = max(w, h)
        guard longEdge > maxLongEdge else { return source }

        let scale = maxLongEdge / longEdge
        let newW  = Int(w * scale)
        let newH  = Int(h * scale)

        let ctx = CGContext(
            data: nil,
            width: newW, height: newH,
            bitsPerComponent: source.bitsPerComponent,
            bytesPerRow: 0,
            space: source.colorSpace ?? CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: source.bitmapInfo.rawValue
        )
        ctx?.draw(source, in: CGRect(x: 0, y: 0, width: newW, height: newH))
        return ctx?.makeImage() ?? source
    }

    // MARK: - PNG encode in-memory → base64

    private func pngBase64(_ image: CGImage) -> String? {
        let data = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil) else {
            return nil
        }
        CGImageDestinationAddImage(dest, image, nil)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return (data as Data).base64EncodedString()
    }

    // MARK: - On-device OCR (Vision)

    private func recognizeText(in image: CGImage) async -> String {
        await withCheckedContinuation { continuation in
            let request = VNRecognizeTextRequest { req, _ in
                let text = (req.results as? [VNRecognizedTextObservation])?
                    .compactMap { $0.topCandidates(1).first?.string }
                    .joined(separator: "\n") ?? ""
                continuation.resume(returning: text)
            }
            request.recognitionLanguages = ["sk-SK", "en-US"]
            request.recognitionLevel     = .accurate
            request.usesLanguageCorrection = true

            let handler = VNImageRequestHandler(cgImage: image, options: [:])
            do {
                try handler.perform([request])
            } catch {
                continuation.resume(returning: "")
            }
        }
    }

    // MARK: - Frontmost app

    private func frontmostApp() async -> String? {
        guard let app = NSWorkspace.shared.frontmostApplication else { return nil }
        let name   = app.localizedName ?? "Unknown"
        let bundle = app.bundleIdentifier.map { " (\($0))" } ?? ""
        return "\(name)\(bundle)"
    }

    // MARK: - Accessibility selected text

    private func selectedText() async -> String? {
        guard AXIsProcessTrusted() else { return nil }

        let systemWide = AXUIElementCreateSystemWide()

        var focusedEl: AnyObject?
        let focusedResult = AXUIElementCopyAttributeValue(
            systemWide,
            kAXFocusedUIElementAttribute as CFString,
            &focusedEl
        )
        guard focusedResult == .success, let element = focusedEl else { return nil }

        // Safety: skip password fields (kAXSecureTextFieldSubrole)
        var subroleRef: AnyObject?
        AXUIElementCopyAttributeValue(
            element as! AXUIElement,
            kAXSubroleAttribute as CFString,
            &subroleRef
        )
        if let subrole = subroleRef as? String, subrole == kAXSecureTextFieldSubrole as String {
            return nil
        }

        var selectedRef: AnyObject?
        let selResult = AXUIElementCopyAttributeValue(
            element as! AXUIElement,
            kAXSelectedTextAttribute as CFString,
            &selectedRef
        )
        guard selResult == .success, let text = selectedRef as? String, !text.isEmpty else {
            return nil
        }
        return text
    }
}
