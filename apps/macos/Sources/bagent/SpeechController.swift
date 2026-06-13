import AVFoundation
import CoreML
import Foundation
import SwiftUI
// WhisperKit isn't fully Sendable-annotated yet; @preconcurrency downgrades its
// non-Sendable cross-actor sends (pipeline components passed to the actor-isolated
// AudioStreamTranscriber init) from errors to warnings under Swift 6.
@preconcurrency import WhisperKit

/// On-device speech-to-text driven by WhisperKit (CoreML / Apple Neural Engine).
///
/// WhisperKit's `AudioStreamTranscriber` owns the microphone capture (via its
/// `AudioProcessor`) and surfaces both the live transcript and a rolling
/// `bufferEnergy` array — we reuse that energy as the waveform amplitude so there
/// is a single audio path. The final transcript is plain text and is handed to
/// `onFinalTranscript`; nothing audio-related ever leaves this process.
///
/// Everything is `@MainActor`: `AudioStreamTranscriber` and the WhisperKit
/// pipeline components are reference types that are not `Sendable`, so confining
/// them to the main actor avoids cross-actor sends under Swift 6 strict
/// concurrency. WhisperKit offloads inference to its own Core ML queues, so the
/// main-actor stream coroutine mostly suspends at `await` points.
@MainActor
final class SpeechController: ObservableObject {

    enum State: Equatable {
        case idle
        case loadingModel
        case listening
        case finalizing
        case done
        case error(String)
    }

    enum Mode { case overlay, inline }

    /// Fuzzy variant name; WhisperKit resolves it to the full CoreML model.
    /// `large-v3-turbo`: highest-accuracy ANE model, comfortably real-time on M-series.
    static let modelVariant = "large-v3-v20240930_turbo"

    @Published private(set) var state: State = .idle
    /// Smoothed 0…1 amplitude for the waveform animation.
    @Published private(set) var amplitude: CGFloat = 0
    /// Running transcript (confirmed + in-flight). Inline mode mirrors this into the text field.
    @Published private(set) var partialText: String = ""
    /// Last ~2 sentences, for the voice-overlay fade/blend display.
    @Published private(set) var sentences: [String] = []
    /// True while first-run model download/load is in progress.
    @Published private(set) var isModelLoaded: Bool = false

    /// Fired once per session with the trimmed final transcript (only if non-empty).
    var onFinalTranscript: ((String) -> Void)?

    /// One transcription update, reduced to Sendable primitives so the WhisperKit
    /// callback never has to capture the (main-actor) controller.
    private struct Frame: Sendable {
        let energy: Float
        let confirmed: [String]
        let unconfirmed: [String]
        let current: String
    }

    private var whisperKit: WhisperKit?
    private var transcriber: AudioStreamTranscriber?
    private var streamTask: Task<Void, Never>?
    private var vadTask: Task<Void, Never>?
    private var frameTask: Task<Void, Never>?
    private var frameContinuation: AsyncStream<Frame>.Continuation?
    private var mode: Mode = .overlay

    // MARK: Silence VAD (session-level auto-stop)
    private var hasHeardSpeech = false
    private var lastVoiceAt: Date?
    /// Relative-energy gate above which we consider the user to be speaking.
    private let voiceThreshold: Float = 0.15
    /// Quiet duration after speech that triggers auto-finalize.
    private let silenceTimeout: TimeInterval = 1.2

    var isRunning: Bool { state == .listening || state == .finalizing || state == .loadingModel }

    // MARK: - Session lifecycle

    func startSession(mode: Mode) async {
        guard !isRunning else { return }
        self.mode = mode
        reset()

        guard await ensureMicPermission() else {
            state = .error("Microphone access denied")
            return
        }
        do {
            try await ensureModelLoaded()
            try startTranscriber()
            state = .listening
            startVAD()
        } catch {
            state = .error(error.localizedDescription)
            stopInternal()
        }
    }

    /// Stop capture and emit the final transcript (auto-stop on silence, or manual).
    func finalize() {
        guard state == .listening || state == .finalizing else { return }
        state = .finalizing
        stopInternal()
        let text = partialText.trimmingCharacters(in: .whitespacesAndNewlines)
        state = .done
        if !text.isEmpty { onFinalTranscript?(text) }
    }

    /// Abort with no transcript (Escape / click-away).
    func cancel() {
        stopInternal()
        reset()
        state = .idle
    }

    // MARK: - Permission

    func ensureMicPermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
        default: return false
        }
    }

    // MARK: - Model

    /// Stable, persistent location for the downloaded CoreML model so it is
    /// reused across launches instead of re-downloading every time.
    private static func modelDownloadBase() -> URL {
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("bagent", isDirectory: true)
            .appendingPathComponent("whisperkit", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base
    }

    private func ensureModelLoaded() async throws {
        if let wk = whisperKit, wk.tokenizer != nil { isModelLoaded = true; return }
        state = .loadingModel
        // Compute units: the Neural Engine gives the best inference perf but its
        // first-load compilation of large-v3 takes minutes, and a re-signed dev
        // binary (`make run`) keeps missing that cache and recompiling. In DEBUG
        // load on CPU+GPU (near-instant) for fast iteration; the RELEASE bundle
        // keeps the ANE path.
        #if DEBUG
        let compute = ModelComputeOptions(
            audioEncoderCompute: .cpuAndGPU,
            textDecoderCompute: .cpuAndGPU
        )
        #else
        let compute = ModelComputeOptions()  // defaults to cpuAndNeuralEngine
        #endif
        // Pin downloadBase so WhisperKit finds the cached model on relaunch.
        let config = WhisperKitConfig(
            model: Self.modelVariant,
            downloadBase: Self.modelDownloadBase(),
            computeOptions: compute
        )
        let wk = try await WhisperKit(config)
        if wk.tokenizer == nil { try await wk.loadModels() }
        whisperKit = wk
        isModelLoaded = true
    }

    // MARK: - Streaming

    private func startTranscriber() throws {
        guard let wk = whisperKit, let tokenizer = wk.tokenizer else {
            throw SpeechError.modelNotLoaded
        }
        var options = DecodingOptions()
        options.task = .transcribe
        options.language = "en"
        options.skipSpecialTokens = true

        // The callback runs in the transcriber's actor domain. It captures only a
        // Sendable continuation (not `self`), so it is provably safe to send into
        // the actor-isolated initializer. We then consume frames on the main actor.
        let (stream, continuation) = AsyncStream.makeStream(of: Frame.self)
        frameContinuation = continuation
        let callback: @Sendable (AudioStreamTranscriber.State, AudioStreamTranscriber.State) -> Void = { _, newState in
            continuation.yield(Frame(
                energy: newState.bufferEnergy.last ?? 0,
                confirmed: newState.confirmedSegments.map(\.text),
                unconfirmed: newState.unconfirmedSegments.map(\.text),
                current: newState.currentText
            ))
        }

        let t = AudioStreamTranscriber(
            audioEncoder: wk.audioEncoder,
            featureExtractor: wk.featureExtractor,
            segmentSeeker: wk.segmentSeeker,
            textDecoder: wk.textDecoder,
            tokenizer: tokenizer,
            audioProcessor: wk.audioProcessor,
            decodingOptions: options,
            useVAD: true,
            stateChangeCallback: callback
        )
        transcriber = t

        frameTask = Task { @MainActor [weak self] in
            for await f in stream {
                self?.ingest(energy: f.energy, confirmed: f.confirmed,
                             unconfirmed: f.unconfirmed, current: f.current)
            }
        }

        streamTask = Task { @MainActor [weak self] in
            do {
                try await t.startStreamTranscription()
            } catch is CancellationError {
                // normal teardown
            } catch {
                self?.state = .error(error.localizedDescription)
            }
        }
    }

    private func ingest(energy: Float, confirmed: [String], unconfirmed: [String], current: String) {
        // Smooth the waveform amplitude.
        let target = CGFloat(min(1, max(0, energy)))
        amplitude += (target - amplitude) * 0.35

        // Session-level voice activity for silence auto-stop.
        if energy > voiceThreshold {
            hasHeardSpeech = true
            lastVoiceAt = Date()
        }

        let confirmedText = confirmed.joined().trimmingCharacters(in: .whitespaces)
        let tail = unconfirmed.joined().trimmingCharacters(in: .whitespaces)
        let live = current.trimmingCharacters(in: .whitespaces)
        let combined = [confirmedText, tail.isEmpty ? live : tail]
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        partialText = combined
        sentences = Self.lastSentences(combined, count: 2)
    }

    private func startVAD() {
        hasHeardSpeech = false
        lastVoiceAt = nil
        vadTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(150))
                guard let self else { return }
                guard self.state == .listening, self.hasHeardSpeech,
                      let last = self.lastVoiceAt else { continue }
                if Date().timeIntervalSince(last) > self.silenceTimeout {
                    self.finalize()
                    break
                }
            }
        }
    }

    // MARK: - Teardown

    private func stopInternal() {
        vadTask?.cancel(); vadTask = nil
        streamTask?.cancel(); streamTask = nil
        frameContinuation?.finish(); frameContinuation = nil
        frameTask?.cancel(); frameTask = nil
        // AudioStreamTranscriber is an actor — stop on its own context. It is
        // Sendable, so capturing it into a detached task is safe.
        if let t = transcriber {
            Task { await t.stopStreamTranscription() }
        }
        transcriber = nil
    }

    private func reset() {
        amplitude = 0
        partialText = ""
        sentences = []
        hasHeardSpeech = false
        lastVoiceAt = nil
    }

    // MARK: - Helpers

    /// Split on sentence terminators and return the last `count` non-empty sentences.
    static func lastSentences(_ text: String, count: Int) -> [String] {
        guard !text.isEmpty else { return [] }
        var result: [String] = []
        var current = ""
        for ch in text {
            current.append(ch)
            if ch == "." || ch == "!" || ch == "?" {
                let trimmed = current.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { result.append(trimmed) }
                current = ""
            }
        }
        let trailing = current.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trailing.isEmpty { result.append(trailing) }
        return Array(result.suffix(count))
    }
}

enum SpeechError: LocalizedError {
    case modelNotLoaded
    var errorDescription: String? {
        switch self {
        case .modelNotLoaded: return "Speech model not loaded"
        }
    }
}
