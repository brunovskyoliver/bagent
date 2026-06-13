import SwiftUI

/// Siri-style layered sine waves driven by live microphone amplitude.
///
/// Uses `TimelineView(.animation)` + `Canvas` (a justified deviation from the
/// project's `@State`-spring house style — continuous wave motion needs a frame
/// clock). Honors Reduce Motion with a static amplitude bar.
struct SiriWaveView: View {
    /// 0…1 smoothed amplitude from `SpeechController`.
    var amplitude: CGFloat
    var isActive: Bool

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let palette: [Color] = [
        Color(red: 0.36, green: 0.42, blue: 0.99),
        Color(red: 0.55, green: 0.36, blue: 0.98),
        Color(red: 0.20, green: 0.78, blue: 0.99),
    ]

    var body: some View {
        if reduceMotion {
            Capsule()
                .fill(palette[0].opacity(0.5))
                .frame(height: 4 + amplitude * 18)
                .frame(maxWidth: .infinity)
        } else {
            TimelineView(.animation) { timeline in
                Canvas { ctx, size in
                    let t = timeline.date.timeIntervalSinceReferenceDate
                    drawWaves(in: &ctx, size: size, time: t)
                }
            }
        }
    }

    private func drawWaves(in ctx: inout GraphicsContext, size: CGSize, time: TimeInterval) {
        let midY = size.height / 2
        // Keep a gentle idle ripple even when quiet; grow with amplitude.
        let amp = max(0.06, amplitude)
        let layers = 3
        for i in 0..<layers {
            var path = Path()
            let phase = time * (1.1 + Double(i) * 0.35) + Double(i) * 1.7
            let layerAmp = midY * (0.18 + amp * 0.7) * (1.0 - CGFloat(i) * 0.18)
            let wavelength = Double(size.width) / (1.2 + Double(i) * 0.5)
            path.move(to: CGPoint(x: 0, y: midY))
            var x: CGFloat = 0
            while x <= size.width {
                let rel = Double(x) / wavelength
                let y = midY + CGFloat(sin(rel * 2 * .pi + phase)) * layerAmp
                path.addLine(to: CGPoint(x: x, y: y))
                x += 2
            }
            path.addLine(to: CGPoint(x: size.width, y: size.height))
            path.addLine(to: CGPoint(x: 0, y: size.height))
            path.closeSubpath()
            let opacity = max(0.08, 0.30 - Double(i) * 0.06)
            ctx.fill(path, with: .color(palette[i % palette.count].opacity(opacity)))
        }
    }
}
