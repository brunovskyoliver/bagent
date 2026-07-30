import AppKit

let arguments = CommandLine.arguments
if arguments.count == 4, arguments[1] == "--stage8-acceptance-chat" {
    let prompt = arguments[2]
    let outputURL = URL(fileURLWithPath: arguments[3])
    Task {
        let status = await Stage8AcceptanceCLI.run(prompt: prompt, outputURL: outputURL)
        exit(status)
    }
    RunLoop.main.run()
} else {
    // Retain delegate for the lifetime of the process.
    let delegate = AppDelegate()
    NSApplication.shared.delegate = delegate
    NSApp.run()
    withExtendedLifetime(delegate) {}
}
