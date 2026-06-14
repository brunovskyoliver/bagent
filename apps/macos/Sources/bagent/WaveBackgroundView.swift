import SwiftUI

// MARK: - File-level palette helpers (shared by the live and static-fallback views)

private struct RGB {
    var r, g, b: Double
    func lerp(to o: RGB, t: Double) -> RGB {
        RGB(r: r + (o.r - r) * t, g: g + (o.g - g) * t, b: b + (o.b - b) * t)
    }
    func color(opacity: Double = 1) -> Color {
        Color(red: r, green: g, blue: b, opacity: opacity)
    }
}

private let wavePalette: [RGB] = [
    RGB(r: 0.36, g: 0.42, b: 0.99),   // electric blue
    RGB(r: 0.20, g: 0.78, b: 0.99),   // cyan
    RGB(r: 0.18, g: 0.92, b: 0.78),   // teal / aqua
    RGB(r: 0.55, g: 0.36, b: 0.98),   // purple
    RGB(r: 0.72, g: 0.28, b: 0.98),   // magenta-violet
    RGB(r: 0.42, g: 0.22, b: 0.98),   // deep violet
]

/// Smooth color interpolation across the full palette (t in 0…1).
private func wavePalColor(_ t: Double, opacity: Double = 1) -> Color {
    let c      = Swift.max(0.0, Swift.min(1.0, t))
    let scaled = c * Double(wavePalette.count - 1)
    let lo     = Swift.min(Int(scaled), wavePalette.count - 2)
    let frac   = scaled - Double(lo)
    return wavePalette[lo].lerp(to: wavePalette[lo + 1], t: frac).color(opacity: opacity)
}

// MARK: - WaveBackgroundView

/// Audio-reactive voice wave visualization.
///
/// Renders a dense stack of flowing curved waveforms, each driven by a frequency
/// band from the live FFT spectrum.  Global amplitude drives glow intensity, wave
/// displacement, phase speed, and particle brightness so the animation feels alive
/// and continuously generated from the incoming audio signal.
///
/// Sizes automatically: a small canvas (e.g. the 34 pt notch strip) gets ~4–6 tight
/// waves; the full 440×190 overlay gets 15+ richly layered waves plus a particle field.
///
/// Public interface is backwards-compatible with the previous `WaveBackgroundView`.
/// `spectrum` is the new optional input; omitting it triggers an amplitude-only fallback.
///
/// Uses `TimelineView(.animation)` + `Canvas` for continuous flowing motion.
/// Honors Reduce Motion with a denser static dotted-curve fallback.
struct WaveBackgroundView: View {
    /// 0…1 smoothed amplitude from `SpeechController`.
    var amplitude: CGFloat
    var isActive: Bool
    /// Minimum wave count hint (preserved for call-site compatibility; actual count
    /// auto-scales with canvas height).
    var bandCount: Int = 4
    /// Live frequency spectrum — `SpectrumAnalyzer.bandCount` values in 0…1.
    /// Pass `speech.spectrum`; leave empty for amplitude-only fallback.
    var spectrum: [CGFloat] = []
    /// Multiplies wave displacement — use > 1 in small containers (e.g. notch strip)
    /// so waves remain visible at low microphone amplitude.
    var displacementBoost: CGFloat = 1.0

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // MARK: - Body

    var body: some View {
        if reduceMotion {
            StaticWaveFallback(amplitude: amplitude, spectrum: spectrum)
        } else {
            TimelineView(.animation) { timeline in
                Canvas { ctx, size in
                    let t = timeline.date.timeIntervalSinceReferenceDate
                    draw(ctx: &ctx, size: size, time: t)
                }
            }
        }
    }

    // MARK: - Main draw pass

    private func draw(ctx: inout GraphicsContext, size: CGSize, time: Double) {
        let energy    = Swift.max(0.03, Double(amplitude))
        let eNorm     = Swift.min(1.0,  energy)
        let eSq       = eNorm * eNorm

        // Auto-scaled wave count from canvas height; bandCount as floor
        let waveCount = Swift.min(16, Swift.max(bandCount, Int(size.height / 11)))

        // Global energy → visual parameters
        let phaseSpeed = 0.18 + eNorm * 0.60
        let glowW      = CGFloat(2.0 + eSq * 9.0)
        let glowAlpha  = CGFloat(0.05 + eSq * 0.30)
        let coreAlpha  = CGFloat(0.28 + eNorm * 0.55)
        let fillAlpha  = CGFloat(0.02 + eSq * 0.06)

        // Expanding pulse ring — subtle energy indicator
        if eNorm > 0.1 && size.height > 40 {
            let pulseR = CGFloat(eNorm) * size.height * 0.65
            let pulseO = CGFloat(eSq * 0.18)
            let center = CGPoint(x: size.width * 0.5, y: size.height * 0.5)
            let pulse  = Path(ellipseIn: CGRect(x: center.x - pulseR, y: center.y - pulseR,
                                                width: pulseR * 2, height: pulseR * 2))
            ctx.stroke(pulse, with: .color(wavePalColor(eNorm * 0.6, opacity: Double(pulseO))),
                       lineWidth: 1.5)
        }

        // Wave stack — back to front
        for i in 0..<waveCount {
            let t     = Double(i) / Double(Swift.max(1, waveCount - 1))
            let color = wavePalColor(t)

            // Frequency band energy driving this wave
            let specVal: Double
            if !spectrum.isEmpty {
                let idx = Swift.min(Int(t * Double(spectrum.count)), spectrum.count - 1)
                specVal = Double(spectrum[idx])
            } else {
                // Amplitude-only fallback: synthesize time-varying pseudo-bands
                let ph = time * (0.25 + t * 0.18) + Double(i) * 1.37
                specVal = eNorm * Swift.max(0, 0.4 + 0.6 * sin(ph))
            }

            // Wave center Y — spread across nearly full height
            let centerY = size.height * CGFloat(0.05 + t * 0.90)

            // Displacement: ambient always-on + band reactive + global energy.
            // Siri-style envelope: waves near t=0.5 (center) are tallest; edge waves
            // taper to ~30% height, creating the characteristic bulging-center look.
            let ambient  = size.height * 0.025
            let reactive = size.height * (0.03 + CGFloat(specVal) * 0.32 + CGFloat(eNorm) * 0.08)
            let siriEnvelope = CGFloat(sin(t * .pi))   // 0 → 1 → 0 across wave stack
            let envScale = 0.30 + siriEnvelope * 0.70  // 0.30 at edges, 1.0 at center
            let dispAmp  = (ambient + reactive) * envScale * displacementBoost

            // Unique phase — avoid lockstep across waves
            let wavePhase = time * phaseSpeed * (0.65 + t * 0.70) + t * .pi * 4.3

            let path = wavePath(size: size, centerY: centerY, dispAmp: dispAmp,
                                phase: wavePhase, specFraction: t)

            // Under-fill (very low opacity — adds depth)
            var fillPath = path
            fillPath.addLine(to: CGPoint(x: size.width, y: size.height))
            fillPath.addLine(to: CGPoint(x: 0,          y: size.height))
            fillPath.closeSubpath()
            ctx.fill(fillPath, with: .color(color.opacity(Double(fillAlpha))))

            // Glow stroke (wide, transparent)
            ctx.stroke(path, with: .color(color.opacity(Double(glowAlpha))), lineWidth: glowW)

            // Core stroke (thin, bright)
            ctx.stroke(path, with: .color(color.opacity(Double(coreAlpha))), lineWidth: 1.2)
        }

        // Particle field — skipped in the tiny 34 pt notch strip
        if size.height > 40 {
            drawParticles(ctx: &ctx, size: size, time: time, eNorm: CGFloat(eNorm))
        }
    }

    // MARK: - Wave path construction

    private func wavePath(
        size: CGSize, centerY: CGFloat, dispAmp: CGFloat,
        phase: Double, specFraction: Double
    ) -> Path {
        var path = Path()
        let step: CGFloat = 3
        var first = true
        var x: CGFloat = 0
        while x <= size.width + step {
            let u = Double(x) / Double(size.width)

            // Primary sine
            let primary   = sin(u * .pi * 5.0 + phase)

            // Secondary harmonic — adds organic texture
            let secondary = sin(u * .pi * 11.3 + phase * 1.42 + specFraction * 2.1) * 0.22

            // Spectrum-driven deformation: different tones produce different shapes
            let specDeform: Double
            if !spectrum.isEmpty {
                let specBin   = u * Double(spectrum.count - 1)
                let loIdx     = Int(specBin)
                let hiIdx     = Swift.min(loIdx + 1, spectrum.count - 1)
                let frac      = specBin - Double(loIdx)
                let localBand = (1 - frac) * Double(spectrum[loIdx]) + frac * Double(spectrum[hiIdx])
                specDeform    = localBand * sin(u * .pi * 8.7 + phase * 0.79) * 0.55
            } else {
                specDeform = 0
            }

            let y = centerY + CGFloat(primary + secondary + specDeform) * dispAmp

            if first { path.move(to: CGPoint(x: x, y: y)); first = false }
            else      { path.addLine(to: CGPoint(x: x, y: y)) }
            x += step
        }
        return path
    }

    // MARK: - Particle field

    private func drawParticles(
        ctx: inout GraphicsContext, size: CGSize, time: Double, eNorm: CGFloat
    ) {
        let step   = CGFloat(13)
        let radius = CGFloat(0.9 + eNorm * 1.4)
        let speed  = CGFloat(0.8 + eNorm * 3.0)
        let drift  = CGFloat(time * Double(speed)).truncatingRemainder(dividingBy: step)
        let base   = CGFloat(0.06 + eNorm * 0.14)

        var dy: CGFloat = 0
        while dy <= size.height {
            let rowOff: CGFloat = (Int(dy / step) % 2 == 0) ? 0 : step * 0.5
            var dx: CGFloat = -step
            while dx <= size.width + step {
                let cx = dx + rowOff - drift
                let flicker = sin(Double(cx * 0.09 + dy * 0.07) + time * 0.4) * 0.5 + 0.5
                let opacity = Double(base * (0.4 + CGFloat(flicker) * 0.6))
                // Hue follows horizontal position across the palette
                let hue      = Swift.max(0, Swift.min(1, Double(cx / size.width)))
                let dotColor = wavePalColor(hue, opacity: opacity)
                let rect     = CGRect(x: cx - radius, y: dy - radius,
                                      width: radius * 2, height: radius * 2)
                ctx.fill(Path(ellipseIn: rect), with: .color(dotColor))
                dx += step
            }
            dy += step
        }
    }
}

// MARK: - Reduce-motion static fallback

private struct StaticWaveFallback: View {
    var amplitude: CGFloat
    var spectrum: [CGFloat]

    var body: some View {
        Canvas { ctx, size in
            let waveCount = Swift.min(12, Swift.max(4, Int(size.height / 11)))
            let energy    = Swift.max(0.04, Double(amplitude))

            for i in 0..<waveCount {
                let t        = Double(i) / Double(Swift.max(1, waveCount - 1))
                let color    = wavePalColor(t)
                let centerY  = size.height * CGFloat(0.05 + t * 0.90)

                let specVal: Double = spectrum.isEmpty ? energy * 0.5 :
                    Double(spectrum[Swift.min(
                        Int(t * Double(spectrum.count)), spectrum.count - 1
                    )])
                let dispAmp    = size.height * CGFloat(0.03 + specVal * 0.25 + energy * 0.06)
                let wavelength = size.width / CGFloat(1.2 + t * 0.8)

                var wave = Path()
                var x: CGFloat = 0
                var first = true
                while x <= size.width {
                    let y = centerY + CGFloat(sin(Double(x / wavelength) * 2 * .pi)) * dispAmp
                    if first { wave.move(to: CGPoint(x: x, y: y)); first = false }
                    else      { wave.addLine(to: CGPoint(x: x, y: y)) }
                    x += 2
                }

                ctx.stroke(wave,
                           with: .color(color.opacity(0.12 + energy * 0.18)),
                           lineWidth: 3.0)
                ctx.stroke(wave,
                           with: .color(color.opacity(0.22 + energy * 0.30)),
                           lineWidth: 1.0)
            }
        }
    }
}
