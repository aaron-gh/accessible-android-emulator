import AppKit

/// Quitting waits for devices that are stopping. A stop first has Android
/// write its data to disk, then tells the emulator to save and exit; quitting
/// before that would leave the emulator running.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: AppModel?

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let model, !model.stoppingNames.isEmpty else { return .terminateNow }
        let names = model.stoppingNames.joined(separator: " and ")
        model.announce("Waiting for \(names) to stop before quitting.")
        model.quitWhenStopped = {
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}
