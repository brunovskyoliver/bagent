import AppKit

let arguments = CommandLine.arguments
if (arguments.count == 3 || arguments.count == 4),
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
} else if (2...3).contains(arguments.count),
          arguments[1] == "--stage5-notch-fixture",
          ProcessInfo.processInfo.environment["BAGENT_STAGE5_ACCEPTANCE_FIXTURE"] == "1" {
    Stage5AcceptanceFixture.run(variant: arguments.count == 3 ? arguments[2] : "default")
} else {
    // Retain delegate for the lifetime of the process.
    let delegate = AppDelegate()
    NSApplication.shared.delegate = delegate
    NSApp.run()
    withExtendedLifetime(delegate) {}
}
