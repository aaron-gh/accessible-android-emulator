import SwiftUI

/// The device's language, display and accessibility settings, each changed
/// as soon as it's chosen, without going through Android's Settings.
struct DeviceSettingsView: View {
    @EnvironmentObject var model: AppModel
    @State private var tags = ""

    var body: some View {
        Form {
            Section {
                ForEach(model.deviceSettings, id: \.name) { setting in
                    Picker(setting.label, selection: Binding(
                        get: { setting.value },
                        set: { model.changeDeviceSetting(setting.name, to: $0) }
                    )) {
                        ForEach(setting.choices, id: \.value) { Text($0.label).tag($0.value) }
                    }
                }
                if model.deviceSettings.isEmpty {
                    Text(model.selected?.running == true ? "Reading the device's settings." : "Start the device to change its settings.")
                }
            }
            Section {
                TextField("Language tags, such as fr-CA, or fr-FR,en-US", text: $tags)
                    .onSubmit { setTags() }
                Button("Set Language") { setTags() }
                    .disabled(tags.trimmingCharacters(in: .whitespaces).isEmpty)
            } header: {
                Text("Another Language").accessibilityAddTraits(.isHeader)
            }
            Section {
                Text(model.newDeviceSettings)
                Button("Use These Settings for New Devices") { model.useSettingsForNewDevices() }
                Button("New Devices Keep Android's Settings") { model.newDevicesKeepAndroidSettings() }
            } header: {
                Text("New Devices").accessibilityAddTraits(.isHeader)
            }
            Section {
                Button("Refresh") { model.loadDeviceSettings() }
                    .keyboardShortcut("r")
            }
        }
        .formStyle(.grouped)
        .disabled(!(model.selected?.running ?? false))
        .frame(minWidth: 460, minHeight: 520)
        .navigationTitle(model.selected.map { "Display and Language: \($0.name)" } ?? "Display and Language")
        .onAppear { model.loadDeviceSettings() }
        .onChange(of: model.selection) { _ in model.loadDeviceSettings() }
    }

    private func setTags() {
        let text = tags.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return }
        model.changeDeviceSetting("language", to: text)
    }
}

/// The Device menu item that opens the Display and Language window.
struct DeviceSettingsMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Display and Language") { openWindow(id: "device-settings") }
            .keyboardShortcut(",", modifiers: [.command, .option])
    }
}
