import SwiftUI

/// Voice-only display mode: pops from the notch, shows the Siri-style waves, an
/// animated `waveform` symbol, and the last ~2 transcribed sentences fading and
/// blending as new ones arrive. Finalizes on silence (handled by SpeechController).
struct VoiceOverlayView: View {
    @ObservedObject var speech: SpeechController
    var onCancel: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var isListening: Bool { speech.state == .listening }

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(.regularMaterial)
                .overlay(
                    RoundedRectangle(cornerRadius: 22, style: .continuous)
                        .strokeBorder(.white.opacity(0.08), lineWidth: 1)
                )

            VStack(spacing: 14) {
                Image(systemName: "waveform")
                    .font(.system(size: 30, weight: .semibold))
                    .foregroundStyle(.tint)
                    // `.repeating` = macOS 14 form of `.repeat(.continuous)`.
                    .symbolEffect(.variableColor.iterative.dimInactiveLayers.reversing,
                                  options: .repeating, isActive: isListening)

                SiriWaveView(amplitude: speech.amplitude, isActive: isListening)
                    .frame(height: 56)
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))

                transcript

                if speech.state == .loadingModel {
                    HStack(spacing: 6) {
                        ProgressView().scaleEffect(0.6)
                        Text("Načítavam Whisper…")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
            }
            .padding(18)
        }
        .frame(width: 360, height: 240)
        .onExitCommand { onCancel() }
    }

    private var transcript: some View {
        VStack(spacing: 6) {
            if speech.sentences.isEmpty {
                Text(promptText)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .transition(.opacity)
            } else {
                ForEach(Array(speech.sentences.enumerated()), id: \.element) { idx, sentence in
                    let isLast = idx == speech.sentences.count - 1
                    Text(sentence)
                        .font(.system(size: isLast ? 15 : 12, weight: isLast ? .medium : .regular))
                        .foregroundStyle(isLast ? Color.primary : Color.secondary)
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                        .id(sentence)
                        .transition(.asymmetric(
                            insertion: .opacity.combined(with: .move(edge: .bottom)),
                            removal: .opacity.combined(with: .move(edge: .top))
                        ))
                }
            }
        }
        .frame(maxWidth: .infinity)
        .animation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.78),
                   value: speech.sentences)
    }

    private var promptText: String {
        switch speech.state {
        case .listening:    return "Počúvam…"
        case .loadingModel: return "Pripravujem…"
        case .error(let m): return m
        default:            return "Hovorte"
        }
    }
}
