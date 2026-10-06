import SwiftUI

@main
struct AAEApp: App {
    @StateObject private var model = AppModel()
    @StateObject private var updater = Updater()

    var body: some Scene {
        Window("Accessible Android Emulator", id: "main") {
            ContentView()
                .environmentObject(model)
        }
        .commands {
            CommandGroup(after: .appInfo) {
                CheckForUpdatesMenuItem(updater: updater)
            }
            CommandGroup(after: .help) {
                SelfTestMenuItem(model: model)
                Button("Save Diagnostic Report…") { model.saveDiagnosticReport() }
            }
            CommandGroup(replacing: .newItem) {
                Button("New Device…") { model.showingNewDevice = true }
                    .keyboardShortcut("n")
                AndroidVersionsMenuItem()
            }
            CommandMenu("Device") {
                Button("Start") { model.start() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
                Button("Stop") { model.stop() }
                    .keyboardShortcut(".", modifiers: [.command, .shift])
                Button("Restart") { model.restart() }
                    .keyboardShortcut("r", modifiers: [.command, .shift])
                DeviceModeMenuItem(model: model)
                Button("Speak Status") { model.speakStatus() }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                InspectorMenuItem()
                SpeechLogMenuItem()
                LogMenuItem()
                ShellMenuItem()
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
                Button("Turn Device Audio Up") { model.stepVolume(up: true) }
                    .keyboardShortcut(.upArrow, modifiers: [.command, .option])
                Button("Turn Device Audio Down") { model.stepVolume(up: false) }
                    .keyboardShortcut(.downArrow, modifiers: [.command, .option])
                Button("Check Audio") { model.checkAudio() }
                    .keyboardShortcut("k", modifiers: [.command, .option])
                Divider()
                Button("Copy Device Clipboard to Mac") { model.copyDeviceClipboard() }
                    .keyboardShortcut("c", modifiers: [.command, .shift])
                Button("Send Mac Clipboard to Device") { model.sendClipboardToDevice() }
                    .keyboardShortcut("v", modifiers: [.command, .shift])
                Button("Type Mac Clipboard on Device") { model.typeClipboard() }
                    .keyboardShortcut("v", modifiers: [.command, .option])
                Divider()
                SnapshotsMenuItem()
                ConditionsMenuItem()
                Divider()
                Button("Install App…") { model.installApp() }
                    .keyboardShortcut("i")
                Button("Install Screen Reader Build…") { model.installScreenReaderBuild() }
                    .keyboardShortcut("i", modifiers: [.command, .shift, .option])
                Button("Save Screenshot…") { model.screenshot() }
                    .keyboardShortcut("p", modifiers: [.command, .shift])
                Divider()
                Button("Rename…") { model.renaming = model.selected }
                    .keyboardShortcut("r")
                Button("Copy…") { model.cloning = model.selected }
                    .keyboardShortcut("d")
                Button("Wipe…") { model.wipe() }
                Button("Delete…") { model.delete() }
                    .keyboardShortcut(.delete)
            }
        }
        Window("Accessibility Inspector", id: "inspector") {
            InspectorView()
                .environmentObject(model)
        }
        Window("Speech Log", id: "speechlog") {
            SpeechLogView()
                .environmentObject(model)
        }
        Window("Device Log", id: "devicelog") {
            LogView()
                .environmentObject(model)
        }
        Window("Android Versions", id: "versions") {
            AndroidVersionsView()
                .environmentObject(model)
        }
        Window("Snapshots", id: "snapshots") {
            SnapshotsView()
                .environmentObject(model)
        }
        Window("Battery, Location and Phone", id: "conditions") {
            ConditionsView()
                .environmentObject(model)
        }
        Window("Self-Test", id: "selftest") {
            SelfTestView()
                .environmentObject(model)
        }
        Window("Shell", id: "shell") {
            ShellView()
                .environmentObject(model)
        }
        Settings {
            SettingsView()
        }
    }
}

/// Device mode captures keys in the main window, so it opens that window
/// first, whichever AAE window the shortcut was pressed in.
struct DeviceModeMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    let model: AppModel

    var body: some View {
        Button("Use Android Keyboard") {
            openWindow(id: "main")
            model.enterDeviceMode()
        }
        .keyboardShortcut("e", modifiers: [.command, .shift])
        Button("Use Gestures") {
            openWindow(id: "main")
            model.enterDeviceMode(gestures: true)
        }
        .keyboardShortcut("g", modifiers: [.command, .shift])
    }
}
