import SwiftUI

@main
struct AAEApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        Window("Accessible Android Emulator", id: "main") {
            ContentView()
                .environmentObject(model)
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Device…") { model.showingNewDevice = true }
                    .keyboardShortcut("n")
            }
            CommandMenu("Device") {
                Button("Start") { model.start() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
                Button("Stop") { model.stop() }
                    .keyboardShortcut(".", modifiers: [.command, .shift])
                Button("Use Android Keyboard") { model.enterDeviceMode() }
                    .keyboardShortcut("e", modifiers: [.command, .shift])
                Button("Speak Status") { model.speakStatus() }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                Divider()
                Button("Back") { model.press("back") }
                    .keyboardShortcut("b", modifiers: [.command, .shift])
                Button("Home") { model.press("home") }
                    .keyboardShortcut("h", modifiers: [.command, .shift])
                Button("Recent Apps") { model.press("recents") }
                    .keyboardShortcut("a", modifiers: [.command, .shift])
                Button("Notifications") { model.showNotifications() }
                    .keyboardShortcut("n", modifiers: [.command, .shift])
                Button("Quick Settings") { model.showQuickSettings() }
                    .keyboardShortcut("q", modifiers: [.command, .shift])
                Divider()
                Button("Rotate Left") { model.rotate(left: true) }
                    .keyboardShortcut(.leftArrow, modifiers: [.command, .shift])
                Button("Rotate Right") { model.rotate(left: false) }
                    .keyboardShortcut(.rightArrow, modifiers: [.command, .shift])
                Button("Mute Device Audio") { model.toggleMute() }
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                Divider()
                Button("Install App…") { model.installApp() }
                    .keyboardShortcut("i")
                Button("Save Screenshot…") { model.screenshot() }
                    .keyboardShortcut("p", modifiers: [.command, .shift])
                Divider()
                Button("Rename…") { model.renaming = model.selected }
                    .keyboardShortcut("r")
                Button("Copy…") { model.cloning = model.selected }
                    .keyboardShortcut("d")
                Button("Delete…") { model.delete() }
                    .keyboardShortcut(.delete)
            }
        }
        Settings {
            SettingsView()
        }
    }
}
