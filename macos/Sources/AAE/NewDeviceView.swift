import SwiftUI
import UniformTypeIdentifiers

/// Creates a device: a name, an Android version, and how to set it up.
struct NewDeviceView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var versionIndex = 0
    @State private var chosenID: String?
    @State private var profile = 1
    @State private var screenReader: String?
    @State private var volumeBoost = true

    private let profiles: [(String, DeviceProfile)] = [
        ("Small phone", .smallPhone),
        ("Phone", .phone),
        ("Tablet", .tablet),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New Device").font(.headline).accessibilityAddTraits(.isHeader)

            if model.versions.isEmpty {
                Text("No Android versions are available. Check your internet connection and try again.")
            } else {
                TextField("Name", text: $name)

                Picker("Android version", selection: $versionIndex) {
                    ForEach(model.versions.indices, id: \.self) { index in
                        Text(versionLabel(model.versions[index])).tag(index)
                    }
                }
                .onChange(of: versionIndex) { index in
                    if model.versions.indices.contains(index) {
                        chosenID = model.versions[index].id
                    }
                }
                // When the list changes, such as with previews turned on, the
                // version chosen stays chosen.
                .onChange(of: model.versions.map(\.id)) { ids in
                    versionIndex = chosenID.flatMap { ids.firstIndex(of: $0) }
                        ?? model.versions.firstIndex(where: \.installed) ?? 0
                }

                Toggle("Include previews of upcoming Android releases", isOn: $model.includePreviews)

                Picker("Size", selection: $profile) {
                    ForEach(profiles.indices, id: \.self) { index in
                        Text(profiles[index].0).tag(index)
                    }
                }

                HStack {
                    Text(screenReaderText)
                        .accessibilityLabel("Screen reader: \(screenReaderText)")
                    Button("Choose Screen Reader…", action: chooseScreenReader)
                }

                Toggle("Turn the screen reader's volume up to full", isOn: $volumeBoost)
            }

            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create and Start", action: create)
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || model.versions.isEmpty)
            }
        }
        .padding()
        .frame(width: 480)
        .onAppear {
            screenReader = model.defaultScreenReader
            // Start on the newest installed version, so nothing downloads by surprise.
            versionIndex = model.versions.firstIndex(where: \.installed) ?? 0
            model.loadVersions()
        }
    }

    private func versionLabel(_ version: VersionInfo) -> String {
        version.installed ? "\(version.description), installed" : "\(version.description), \(version.size) to download"
    }

    private var screenReaderText: String {
        guard let screenReader else { return "The Android image's own, if it has one. If not, AAE asks." }
        return (screenReader as NSString).lastPathComponent
    }

    private func chooseScreenReader() {
        let panel = NSOpenPanel()
        panel.message = "Choose a screen reader APK, such as a Backtalk build."
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data]
        if panel.runModal() == .OK, let url = panel.url {
            screenReader = url.path
        }
    }

    private func create() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, model.versions.indices.contains(versionIndex) else { return }
        dismiss()
        model.create(
            name: trimmed,
            version: model.versions[versionIndex],
            profile: profiles[profile].1,
            screenReader: screenReader,
            volumeBoost: volumeBoost
        )
    }
}
