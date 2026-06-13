# Spike: Local Whisper Speech-to-Text (Voice Input)

**Date:** 2026-06-13
**Device:** MacBook Pro Mac17,2, Apple M5, 32 GB RAM
**Scope:** On-device, English-only voice input. Transcript enters the existing `/chat` pipeline as plain text.

---

## Decision: WhisperKit (CoreML / ANE) in Swift

Audio capture must happen in Swift regardless (microphone permission + the amplitude
that drives the waveform). Transcribing in Swift too keeps capture + STT in one process
with the lowest latency for live partials, and the transcript becomes normal text that
enters the **unchanged** daemon `/chat` endpoint.

| | A) Swift + WhisperKit (chosen) | B) Rust + whisper.cpp + Metal |
|---|---|---|
| Audio capture | AVAudioEngine (via WhisperKit's `AudioProcessor`) | Still AVAudioEngine in Swift |
| Live partials | First-class (`AudioStreamTranscriber`) | Hand-rolled chunked streaming + IPC |
| Backend coupling | None — final text → `/chat` | New audio endpoint + audio plumbing |
| Net | One audio path, no IPC | Worst of both: Swift capture **and** Rust service |

Route B would force PCM streaming over IPC while *still* needing AVAudioEngine in Swift.

## Package

```swift
.package(url: "https://github.com/argmaxinc/WhisperKit.git", from: "0.9.0")
// manifest package name is "argmax-oss-swift"; product is "WhisperKit"
```

Minimum macOS 13 (we target 14). Models auto-download from `argmaxinc/whisperkit-coreml`
into Application Support on first use.

## API used (`SpeechController.swift`)

- `WhisperKit(WhisperKitConfig(model: "large-v3-v20240930_turbo"))` → `loadModels()`
- `AudioStreamTranscriber` — **an actor**; owns mic capture via its `AudioProcessor`.
  - `State.bufferEnergy: [Float]` → reused as the waveform amplitude (single audio path).
  - `State.confirmedSegments` / `unconfirmedSegments` / `currentText` → live transcript.
  - `startStreamTranscription() async throws` / `stopStreamTranscription()` (await — actor-isolated).

## Swift 6 strict-concurrency notes (important for future edits)

- `AudioStreamTranscriber` is an actor → `stopStreamTranscription()` must be `await`ed
  from a `Task` (it is `Sendable`, so capturing the actor into a detached task is safe).
- WhisperKit is not fully Sendable-annotated → `@preconcurrency import WhisperKit`
  downgrades non-Sendable component sends (`any TextDecoding`, `any AudioProcessing`,
  tokenizer) into the actor init from errors to warnings.
- The `stateChangeCallback` must **not** capture the `@MainActor` controller. It captures
  only a Sendable `AsyncStream.Continuation` and yields extracted primitives; the main
  actor consumes the stream and updates `@Published` state. This keeps the closure
  provably `Sendable`.

## Model selection & latency (M5, ANE)

- `large-v3-turbo` — highest accuracy, comfortably faster than real-time; ~10 s utterance
  transcribes in well under ~1–2 s; partials update every ~1–2 s.
- ~1.5 GB first-run download. Fallback (deferred): `base.en` partials + turbo final pass.

## Voice activity / auto-stop

Session-level VAD on `bufferEnergy`: once speech is heard (energy > ~0.15), a quiet gap of
~1.2 s triggers `finalize()`. (WhisperKit's internal `useVAD`/`silenceThreshold` only gates
segment confirmation, not session stop.)

## Privacy

100% on-device. Raw PCM lives only in memory during capture and is discarded on finalize;
audio never reaches the daemon. Only the **text transcript** is logged, exactly as typed
text is today (`audit_entries`, no schema change).

## ⚠️ Testing caveat — must run the bundled app

`make run` uses `swift run`, which launches the bare executable **without an Info.plist**,
so `NSMicrophoneUsageDescription` is absent and microphone capture will be denied. Voice
QA must use the bundle:

```bash
cd apps/macos && make bundle && open bagent.app
```

Ad-hoc signing (`codesign --sign -`) means macOS may re-prompt for microphone access after
each rebuild.
