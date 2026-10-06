import SwiftUI

struct ContentView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        Group {
            if let error = model.startupError {
                VStack(alignment: .leading, spacing: 12) {
                    Text("AAE could not start").font(.title2).accessibilityAddTraits(.isHeader)
                    Text(error).textSelection(.enabled)
                }
                .padding()
            } else if model.inDeviceMode {
                DeviceModeView()
            } else {
                DeviceListView()
            }
        }
        .frame(minWidth: 560, minHeight: 380)
        .sheet(isPresented: $model.showingNewDevice) {
            NewDeviceView()
        }
        .sheet(item: $model.renaming) { device in
            NamePrompt(
                title: "Rename \(device.name)",
                label: "New name",
                initial: device.name,
                action: "Rename"
            ) { model.rename(device, to: $0) }
        }
        .sheet(item: $model.cloning) { device in
            NamePrompt(
                title: "Copy \(device.name)",
                label: "Name for the copy",
                initial: "\(device.name) copy",
                action: "Copy"
            ) { model.clone(device, as: $0) }
        }
    }
}

extension DeviceInfo: Identifiable {}

/// The device list and the buttons for the selected device.
struct DeviceListView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.devices.isEmpty {
                Text("You have no devices yet. Choose New Device to create one.")
            } else {
                List(model.devices, selection: $model.selection) { device in
                    Text(rowText(device))
                        .tag(device.id)
                        .accessibilityLabel(rowText(device))
                }
                .accessibilityLabel("Devices")
            }

            HStack {
                Button("New Device…") { model.showingNewDevice = true }
                if let device = model.selected {
                    if model.busy[device.id] != nil {
                        ProgressView().controlSize(.small).accessibilityLabel(model.busy[device.id] ?? "")
                    } else if device.running {
                        Button("Use Android Keyboard") { model.enterDeviceMode() }
                        Button("Stop") { model.stop() }
                    } else {
                        Button("Start") { model.start() }
                    }
                }
            }

            if let device = model.selected, device.running, model.busy[device.id] == nil {
                HStack {
                    Button("Back") { model.press("back") }
                    Button("Home") { model.press("home") }
                    Button("Recent Apps") { model.press("recents") }
                    Button("Notifications") { model.showNotifications() }
                    Button("Install App…") { model.installApp() }
                }
            }

            if !model.status.isEmpty {
                Text(model.status)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .accessibilityLabel("Last message: \(model.status)")
            }
        }
        .padding()
    }

    private func rowText(_ device: DeviceInfo) -> String {
        let state = model.busy[device.id]?.lowercased() ?? (device.running ? "running" : "stopped")
        return "\(device.name), \(device.android), \(device.kind), \(state)"
    }
}

/// Shown while the keyboard belongs to Android.
struct DeviceModeView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ZStack {
            KeyCapture(capturing: model.inDeviceMode) { code, down in
                model.sendKey(code, down)
            }
            VStack(spacing: 8) {
                Text("The keyboard is in \(model.selected?.name ?? "Android").")
                    .font(.title2)
                Text("Press Control Command Escape to return to the Mac.")
            }
            .accessibilityHidden(true)
        }
        .padding()
    }
}

/// Asks for a name: used for renaming and copying.
struct NamePrompt: View {
    let title: String
    let label: String
    let initial: String
    let action: String
    let done: (String) -> Void
    @State private var name = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline).accessibilityAddTraits(.isHeader)
            TextField(label, text: $name)
                .onSubmit(submit)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(action, action: submit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding()
        .frame(width: 380)
        .onAppear { name = initial }
    }

    private func submit() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        dismiss()
        done(trimmed)
    }
}
