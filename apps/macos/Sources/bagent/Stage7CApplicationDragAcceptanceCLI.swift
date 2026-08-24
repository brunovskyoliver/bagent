import AppKit
import Foundation
import UniformTypeIdentifiers

enum Stage7CAcceptanceMarker {
    static func write(_ marker: String) {
        guard let directory = ProcessInfo.processInfo.environment["BAGENT_STAGE7C_EVIDENCE_DIR"] else { return }
        try? FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        try? Data(marker.utf8).write(
            to: URL(fileURLWithPath: directory).appendingPathComponent("\(marker).marker"),
            options: .atomic)
    }
}

enum Stage7CApplicationDragAcceptanceCLI {
    static func run(outputURL: URL) async -> Int32 {
        do {
            let candidate = Bundle.main.bundleURL
            let signature = ApplicationSignatureInspector.inspect(candidate)
            guard ApplicationDragValidator.validate(
                appURL: candidate,
                expectedIdentifier: "sk.bagent.app",
                expectedTeamID: "QUB47S3XTF",
                signature: signature
            ) == .valid,
            let provider = ApplicationDragRepresentation.validatedDragItem(for: candidate),
            provider.registeredTypeIdentifiers.contains(UTType.fileURL.identifier) else {
                throw NSError(domain: "Stage7CApplicationDragAcceptanceCLI", code: 1)
            }

            let roundTrip = try await loadFileURL(from: provider)
            guard roundTrip.standardizedFileURL == candidate.standardizedFileURL else {
                throw NSError(domain: "Stage7CApplicationDragAcceptanceCLI", code: 2)
            }

            let missing = candidate.deletingLastPathComponent().appendingPathComponent("missing.app")
            let rejections: [String: ApplicationDragValidationResult] = [
                "image": ApplicationDragValidator.rejection(
                    for: candidate.deletingLastPathComponent().appendingPathComponent("bagent.png")),
                "executable": ApplicationDragValidator.rejection(
                    for: candidate.appendingPathComponent("Contents/MacOS/bagent")),
                "alias": ApplicationDragValidator.rejection(for: candidate, isAlias: true),
                "source_directory": ApplicationDragValidator.rejection(for: candidate.deletingLastPathComponent()),
                "missing_bundle": ApplicationDragValidator.validate(
                    appURL: missing,
                    expectedIdentifier: "sk.bagent.app",
                    expectedTeamID: "QUB47S3XTF",
                    signature: .invalid),
                "wrong_identity": ApplicationDragValidator.validate(
                    appURL: candidate,
                    expectedIdentifier: "sk.other.app",
                    expectedTeamID: "QUB47S3XTF",
                    signature: signature),
                "ad_hoc_signature": ApplicationDragValidator.validate(
                    appURL: candidate,
                    expectedIdentifier: "sk.bagent.app",
                    expectedTeamID: "QUB47S3XTF",
                    signature: .adHoc),
                "file_promise": ApplicationDragValidator.rejection(for: candidate, isFilePromise: true),
            ]
            guard rejections["image"] == .notApplicationBundle,
                  rejections["executable"] == .notApplicationBundle,
                  rejections["alias"] == .alias,
                  rejections["source_directory"] == .notApplicationBundle,
                  rejections["missing_bundle"] == .missingBundle,
                  rejections["wrong_identity"] == .wrongIdentifier,
                  rejections["ad_hoc_signature"] == .adHocSignature,
                  rejections["file_promise"] == .filePromise else {
                throw NSError(domain: "Stage7CApplicationDragAcceptanceCLI", code: 3)
            }

            let evidence: [String: Any] = [
                "candidate_bundle": candidate.path,
                "bundle_identifier": signature.identifier ?? "",
                "team_identifier": signature.teamIdentifier ?? "",
                "registered_type": UTType.fileURL.identifier,
                "round_trip_bundle": roundTrip.path,
                "rejections": rejections.mapValues { String(describing: $0) },
                "system_settings_drop": "omitted",
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: outputURL, options: .atomic)
            return 0
        } catch {
            fputs("Stage 7C application drag fixture failed: \(error)\n", stderr)
            return 1
        }
    }

    private static func loadFileURL(from provider: NSItemProvider) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }
                if let data = item as? Data,
                   let url = URL(dataRepresentation: data, relativeTo: nil) {
                    continuation.resume(returning: url)
                } else if let url = item as? URL {
                    continuation.resume(returning: url)
                } else if let url = item as? NSURL {
                    continuation.resume(returning: url as URL)
                } else {
                    continuation.resume(throwing: NSError(domain: "Stage7CApplicationDragAcceptanceCLI", code: 4))
                }
            }
        }
    }
}
