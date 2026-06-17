import AppKit
import CoreImage
import CoreImage.CIFilterBuiltins

/// Generates a QR code `NSImage` from a string using CoreImage.
enum QRImage {
    /// Returns an upscaled QR NSImage, or nil if generation fails.
    static func generate(from string: String, size: CGFloat = 180) -> NSImage? {
        guard let data = string.data(using: .isoLatin1) else { return nil }

        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")

        guard let ciImage = filter.outputImage else { return nil }

        // Scale up to requested size (QR output is ~21x21 px)
        let scale = size / ciImage.extent.width
        let scaled = ciImage.transformed(by: CGAffineTransform(scaleX: scale, y: scale))

        let rep = NSCIImageRep(ciImage: scaled)
        let img = NSImage(size: rep.size)
        img.addRepresentation(rep)
        return img
    }
}
