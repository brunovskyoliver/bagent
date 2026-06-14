import SwiftUI

// MARK: - Voice content rendered inside the notch bridge during voice mode

struct VoiceNotchContent: View {
    @ObservedObject var speech: SpeechController
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 6) {
            if let msg = viewModel.voiceActionMessage {
                HStack(spacing: 5) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(Color(red: 0.18, green: 0.80, blue: 0.44))
                        .font(.system(size: 12))
                    Text(msg)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                }
                .transition(.opacity.combined(with: .scale(scale: 0.85)))
            } else {
                // Taller wave strip — displacementBoost keeps waves prominent even
                // at low mic amplitude; Siri envelope makes center waves tallest.
                WaveBackgroundView(
                    amplitude: speech.amplitude,
                    isActive: speech.state == .listening,
                    bandCount: 5,
                    spectrum: speech.spectrum,
                    displacementBoost: 3.0
                )
                .frame(height: 52)
                .mask(
                    LinearGradient(
                        stops: [
                            .init(color: .clear, location: 0.00),
                            .init(color: .white, location: 0.14),
                            .init(color: .white, location: 0.86),
                            .init(color: .clear, location: 1.00),
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

                // Live transcript — words animate in one by one
                if let sentence = speech.sentences.last {
                    WordRevealView(sentence: sentence, reduceMotion: reduceMotion)
                        .id(sentenceKey(sentence))
                        .transition(.asymmetric(
                            insertion: .opacity.combined(with: .move(edge: .bottom)),
                            removal:   .opacity.combined(with: .move(edge: .top))
                        ))
                }
            }
        }
        .frame(maxWidth: .infinity)
        .animation(.spring(response: 0.28, dampingFraction: 0.75), value: viewModel.voiceActionMessage)
        .animation(.spring(response: 0.32, dampingFraction: 0.78), value: speech.sentences.count)
    }

    // Use the first 3 words as a stable ID — changes only on true sentence boundary.
    private func sentenceKey(_ s: String) -> String {
        s.split(separator: " ").prefix(3).joined(separator: " ")
    }
}

// MARK: - Word-by-word animated text

/// Reveals words one at a time as `sentence` grows. Resets on a new sentence.
private struct WordRevealView: View {
    let sentence: String
    let reduceMotion: Bool

    @State private var shownText: String = ""
    @State private var revealTask: Task<Void, Never>? = nil

    private func wordList(_ s: String) -> [String] {
        s.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
    }

    var body: some View {
        Text(shownText.isEmpty ? " " : shownText)
            .font(.system(size: 15, weight: .semibold, design: .rounded))
            .foregroundStyle(Color.white.opacity(0.94))
            .multilineTextAlignment(.center)
            .lineLimit(2)
            .frame(maxWidth: .infinity)
            .onAppear { revealFrom(old: "", new: sentence) }
            .onChange(of: sentence) { old, new in revealFrom(old: old, new: new) }
    }

    private func revealFrom(old: String, new: String) {
        let newWords = wordList(new)
        let isNewSentence = !new.hasPrefix(old) && !old.isEmpty

        revealTask?.cancel()

        if reduceMotion {
            shownText = new
            return
        }

        let startIdx: Int
        if isNewSentence {
            shownText = ""
            startIdx = 0
        } else {
            // Continuation — keep already-shown prefix, only animate new words.
            let oldWords = wordList(old)
            startIdx = min(oldWords.count, newWords.count)
            if startIdx > 0 {
                shownText = newWords.prefix(startIdx).joined(separator: " ")
            }
        }

        guard startIdx < newWords.count else { return }

        revealTask = Task { @MainActor in
            for i in startIdx..<newWords.count {
                guard !Task.isCancelled else { break }
                if i > startIdx {
                    try? await Task.sleep(for: .milliseconds(85))
                    guard !Task.isCancelled else { break }
                }
                withAnimation(.easeOut(duration: 0.13)) {
                    shownText = newWords.prefix(i + 1).joined(separator: " ")
                }
            }
        }
    }
}
