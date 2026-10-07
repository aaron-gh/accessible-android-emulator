import SwiftUI

@main
struct AAEApp: App {
    @StateObject private var model = AppModel()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var updater = Updater()
    @StateObject private var serving = Serving()

    var body: some Scene {
        Window("Accessible Android Emulator", id: "main") {
            ContentView()
                .environmentObject(model)
                .onAppear { appDelegate.model = model }
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
                ServeMenuItem()
            }
            CommandMenu("Device") {
                Button("Start") { model.start() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
                Button("Stop") { model.stop() }
                    .keyboardShortcut(".", modifiers: [.command, .shift])
                Button("Restart") { model.restart() }
                    .keyboardShortcut("r", modifiers: [.command, .shift])
                DeviceModeMenuItem(model: model)
                DeviceWindowMenuItem(model: model)
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
                AppsMenuItem()
                ServicesMenuItem()
                Button("Open Link…") { model.showingOpenLink = true }
                    .keyboardShortcut("l", modifiers: [.command, .shift])
                Button("Send Intent…") { model.showingSendIntent = true }
                SnapshotsMenuItem()
                ConditionsMenuItem()
                Divider()
                Button("Install App…") { model.installApp() }
                    .keyboardShortcut("i")
                WatchMenuItems(model: model)
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
        WindowGroup("Device", id: "device", for: String.self) { $id in
            if let id {
                DeviceWindowView(id: id)
                    .environmentObject(model)
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
        Window("Serve Devices to Phones", id: "serving") {
            ServeView()
                .environmentObject(serving)
                .onAppear { serving.announce = { text in model.announce(text) } }
        }
        Window("Watching for New Builds", id: "watches") {
            WatchView()
                .environmentObject(model)
        }
        Window("Accessibility Services", id: "services") {
            ServicesView()
                .environmentObject(model)
        }
        Window("Apps", id: "apps") {
            AppsView()
                .environmentObject(model)
        }
        Window("Snapshots", id: "snapshots") {
            SnapshotsView()
                .environmentObject(model)
        }
        Window("Battery, Location, Phone and Network", id: "conditions") {
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
        Button("Use Android Keyboard") { enter(gestures: false) }
            .keyboardShortcut("e", modifiers: [.command, .shift])
        Button("Use Gestures") { enter(gestures: true) }
            .keyboardShortcut("g", modifiers: [.command, .shift])
    }

    /// In the device's own window if that's in front, otherwise in the main
    /// window, which comes forward.
    private func enter(gestures: Bool) {
        if let id = model.keyDeviceWindow, id == model.selection {
            model.enterDeviceMode(gestures: gestures, host: id)
        } else {
            openWindow(id: "main")
            model.enterDeviceMode(gestures: gestures)
        }
    }
}

/// The Device menu item that gives the selected device a window of its own.
struct DeviceWindowMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    let model: AppModel

    var body: some View {
        Button("Open in Own Window") {
            if let id = model.selection { openWindow(id: "device", value: id) }
        }
        .keyboardShortcut("o", modifiers: [.command, .option])
    }
}
