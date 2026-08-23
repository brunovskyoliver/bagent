import AppKit

let arguments = CommandLine.arguments
#if BAGENT_ACCEPTANCE
func runStage8LiveAcceptanceCommand(_ arguments: [String]) -> Bool {
    if arguments.count == 6, arguments[1] == "--stage8-live-session" {
        let runIdentity = arguments[2]
        let workIdentity = arguments[3]
        guard let workRevision = UInt64(arguments[4]) else { exit(64) }
        let outputURL = URL(fileURLWithPath: arguments[5])
        Task { @MainActor in
            exit(await Stage8LiveSmokeCLI.run(
                runIdentity: runIdentity,
                workIdentity: workIdentity,
                workRevision: workRevision,
                outputURL: outputURL
            ))
        }
        RunLoop.main.run()
        return true
    }
    if arguments.count == 3, arguments[1] == "--stage8-live-projection" {
        let outputURL = URL(fileURLWithPath: arguments[2])
        Task { @MainActor in
            exit(await Stage8LiveSmokeCLI.runProjection(outputURL: outputURL))
        }
        RunLoop.main.run()
        return true
    }
    return false
}

let handledStage8LiveAcceptance = runStage8LiveAcceptanceCommand(arguments)
#else
let handledStage8LiveAcceptance = false
#endif

if handledStage8LiveAcceptance {
    // The acceptance handler owns the main run loop.
} else if (arguments.count == 3 || arguments.count == 4),
   arguments[1] == "--stage7a-relaunch-fixture" {
    let outputURL = URL(fileURLWithPath: arguments[2])
    let sentinelURL = arguments.count == 4 ? URL(fileURLWithPath: arguments[3]) : nil
    Task {
        exit(await Stage7AAcceptanceCLI.run(outputURL: outputURL, sentinelURL: sentinelURL))
    }
    RunLoop.main.run()
} else if arguments.count == 6, arguments[1] == "--stage8-acceptance-case" {
    let acquisition = arguments[2]
    let polish = arguments[3]
    let prompt = arguments[4]
    let outputURL = URL(fileURLWithPath: arguments[5])
    Task {
        let status = await Stage8AcceptanceCLI.run(
            acquisition: acquisition,
            polish: polish,
            prompt: prompt,
            outputURL: outputURL
        )
        exit(status)
    }
    RunLoop.main.run()
} else if (3...4).contains(arguments.count),
          arguments[1] == "--stage7b-settings-fixture",
          ProcessInfo.processInfo.environment[Stage7BSettingsAcceptanceCLI.environmentKey] == "1" {
    let outputDirectory = URL(fileURLWithPath: arguments[2], isDirectory: true)
    let variant = arguments.count == 4 ? arguments[3] : "default"
    Task { @MainActor in
        exit(await Stage7BSettingsAcceptanceCLI.run(outputDirectory: outputDirectory, variant: variant))
    }
    RunLoop.main.run()
} else if arguments.count == 3,
          arguments[1] == "--stage7b-settings-ax-fixture",
          ProcessInfo.processInfo.environment[Stage7BSettingsAcceptanceCLI.accessibilityEnvironmentKey] == "1" {
    let outputDirectory = URL(fileURLWithPath: arguments[2], isDirectory: true)
    Task { @MainActor in
        exit(await Stage7BSettingsAcceptanceCLI.runLiveAccessibility(outputDirectory: outputDirectory))
    }
    RunLoop.main.run()
} else if arguments.count == 3,
          arguments[1] == "--stage8-accessibility-fixture",
          ProcessInfo.processInfo.environment[Stage8AccessibilityCLI.environmentKey] == "1" {
    let outputURL = URL(fileURLWithPath: arguments[2])
    Task { @MainActor in
        exit(await Stage8AccessibilityCLI.run(outputURL: outputURL))
    }
    RunLoop.main.run()
} else if arguments.count == 4,
          arguments[1] == "--stage8-visual-capture",
          ProcessInfo.processInfo.environment[Stage8VisualCaptureCLI.environmentKey] == "1" {
    let outputDirectory = URL(fileURLWithPath: arguments[2], isDirectory: true)
    let evidenceURL = URL(fileURLWithPath: arguments[3])
    Task { @MainActor in
        exit(Stage8VisualCaptureCLI.run(outputDirectory: outputDirectory, evidenceURL: evidenceURL))
    }
    RunLoop.main.run()
} else if arguments.count == 3, arguments[1] == "--stage7c-drag-validation" {
    let outputURL = URL(fileURLWithPath: arguments[2])
    Task {
        exit(await Stage7CApplicationDragAcceptanceCLI.run(outputURL: outputURL))
    }
    RunLoop.main.run()
} else if (2...3).contains(arguments.count),
          arguments[1] == "--stage5-notch-fixture",
          ProcessInfo.processInfo.environment["BAGENT_STAGE5_ACCEPTANCE_FIXTURE"] == "1" {
    Stage5AcceptanceFixture.run(variant: arguments.count == 3 ? arguments[2] : "default")
} else {
    // Retain delegate for the lifetime of the process.
    let delegate = AppDelegate(launchMode: AppLaunchMode.parse(arguments: arguments))
    NSApplication.shared.delegate = delegate
    NSApplication.shared.finishLaunching()
    NSApp.run()
    withExtendedLifetime(delegate) {}
}
