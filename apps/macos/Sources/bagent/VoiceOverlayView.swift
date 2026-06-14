import SwiftUI

/// Voice display panel for **non-notch displays**.
///
/// Drops below the centered menu-bar pill (wired up by `NotchWindowController` when
/// `hasNotch == false`). The wave background fills the panel at low z-order; the
/// animated waveform symbol and live transcript float on top.
struct VoiceOverlayView: View {
    @ObservedObject var speech: SpeechController
    var onCancel: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var isListening: Bool { speech.state == .listening }

    private var overlayIconName: String {
        switch speech.state {
        case .loadingModel: return "waveform.badge.magnifyingglass"
        case .listening:    return "waveform"
        case .finalizing:   return "waveform.badge.clock"
        case .error:        return "exclamationmark.triangle"
        default:            return "waveform"
        }
    }

    var body: some View {
        ZStack {
            // Wave background — fills the whole panel, below everything else.
            // Gradient mask feathers the waves at top & bottom so they dissolve
            // into the background instead of being hard-clipped.
            WaveBackgroundView(
                amplitude: speech.amplitude,
                isActive: isListening,
                bandCount: 4,
                spectrum: speech.spectrum
            )
            .mask(
                LinearGradient(
                    stops: [
                        .init(color: .clear, location: 0.00),
                        .init(color: .white, location: 0.15),
                        .init(color: .white, location: 0.85),
                        .init(color: .clear, location: 1.00),
                    ],
                    startPoint: .top,
                    endPoint: .bottom
                )
            )
            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))

            // Subtle dark overlay so text stays readable above the waves
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(.black.opacity(0.45))

            // Thin border
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(.white.opacity(0.10), lineWidth: 1)

            // Content — layered above waves
            VStack(spacing: 14) {
                // Animated waveform icon — symbol name and animation track speech state.
                Image(systemName: overlayIconName)
                    .font(.system(size: 28, weight: .semibold))
                    .foregroundStyle(.white)
                    .symbolEffect(
                        .variableColor.iterative.dimInactiveLayers.nonReversing,
                        options: .repeating,
                        isActive: isListening
                    )
                    .contentTransition(.symbolEffect(.replace))

                transcript
            }
            .padding(20)
        }
        .frame(width: 440, height: 190)
        .onExitCommand { onCancel() }
    }

    // MARK: - Transcript

    private var transcript: some View {
        VStack(spacing: 6) {
            if speech.sentences.isEmpty {
                // Show error text only — other states communicated by the icon above.
                if case .error(let m) = speech.state {
                    Text(m)
                        .font(.system(size: 12, weight: .regular, design: .rounded))
                        .foregroundStyle(.white.opacity(0.65))
                        .multilineTextAlignment(.center)
                        .transition(.opacity)
                }
            } else {
                ForEach(Array(speech.sentences.enumerated()), id: \.element) { idx, sentence in
                    let isLast = idx == speech.sentences.count - 1
                    Text(sentence)
                        .font(.system(size: isLast ? 15 : 12,
                                      weight: isLast ? .medium : .regular,
                                      design: .rounded))
                        .foregroundStyle(isLast ? Color.white : Color.white.opacity(0.50))
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                        .id(sentence)
                        .transition(.asymmetric(
                            insertion: .opacity.combined(with: .move(edge: .bottom)),
                            removal:   .opacity.combined(with: .move(edge: .top))
                        ))
                }
            }
        }
        .frame(maxWidth: .infinity)
        .animation(
            reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.78),
            value: speech.sentences
        )
    }
}
