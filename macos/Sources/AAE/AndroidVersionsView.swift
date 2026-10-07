import SwiftUI

/// The installed Android versions: how much space each takes, which devices
/// use it, and a way to delete the ones no device needs.
struct AndroidVersionsView: View {
    @EnvironmentObject var model: AppModel
    @State private var selection: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Installed Android Versions")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)

            List(model.installedImages, id: \.sysdir, selection: $selection) { image in
                VStack(alignment: .leading, spacing: 2) {
                    Text(image.description)
                    Text(details(image))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }
            .accessibilityLabel("Android versions")
            .frame(minHeight: 240)
            .overlay {
                if model.installedImages.isEmpty {
                    Text(model.loadingImages ? "Reading…" : "No Android versions are installed.")
                        .foregroundStyle(.secondary)
                }
            }

            Text("Emulator and Tools")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            if let status = model.setupStatus {
                if !status.managedElsewhere.isEmpty {
                    Text("Managed outside AAE: \(status.managedElsewhere.joined(separator: ", ")).")
                }
                if status.updates.isEmpty {
                    if status.managedElsewhere.isEmpty {
                        Text("The emulator and tools are up to date.")
                    }
                } else {
                    Text("Updates: " + status.updates.map { "\($0.name) \($0.revision), \($0.size)" }.joined(separator: "; ") + ".")
                    if let download = model.download {
                        ProgressView(value: Double(download.percent), total: 100) {
                            Text("Updating: \(download.percent)%")
                        }
                    } else {
                        Button("Update the Emulator and Tools") { model.runSetup(update: true) }
                            .disabled(model.settingUp)
                    }
                    Text("Stop every device before updating.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(model.setupError ?? "Checking for updates…")
            }

            HStack {
                Button("Delete…") {
                    if let image = selected { model.removeImage(image) }
                }
                .disabled(selected == nil)
                Spacer()
                Button("Refresh") { model.loadInstalledImages() }
                    .keyboardShortcut("r")
            }
        }
        .padding()
        .frame(minWidth: 560, minHeight: 380)
        .onAppear {
            model.loadInstalledImages()
            model.checkSetup()
        }
    }

    private var selected: InstalledImageInfo? {
        model.installedImages.first { $0.sysdir == selection }
    }

    private func details(_ image: InstalledImageInfo) -> String {
        var parts = [image.size]
        if image.devices.isEmpty {
            parts.append("No AAE devices use it")
        } else {
            parts.append("Used by \(image.devices.joined(separator: ", "))")
        }
        if !image.otherDevices.isEmpty {
            parts.append("Also used by other emulator devices: \(image.otherDevices.joined(separator: ", "))")
        }
        return parts.joined(separator: ". ") + "."
    }
}

/// The File menu item that opens the Android Versions window.
struct AndroidVersionsMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Android Versions") { openWindow(id: "versions") }
            .keyboardShortcut("a", modifiers: [.command, .option])
    }
}
