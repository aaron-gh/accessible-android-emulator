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
    @State private var memory = ""
    @State private var cores = ""
    @State private var storage = ""
    @State private var defaults = (memory: "", cores: "", storage: "")

    private let profiles: [(String, DeviceProfile)] = [
        ("Small phone", .smallPhone),
        ("Phone", .phone),
        ("Tablet", .tablet),
        ("Foldable", .foldable),
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
                        Text(tr(versionLabel(model.versions[index]))).tag(index)
                    }
                }
                .onChange(of: versionIndex) { index in
                    if model.versions.indices.contains(index) {
                        chosenID = model.versions[index].id
                    }
                    loadDefaults()
                }
                // When the list changes, such as with previews turned on, the
                // version chosen stays chosen.
                .onChange(of: model.versions.map(\.id)) { ids in
                    versionIndex = chosenID.flatMap { ids.firstIndex(of: $0) }
                        ?? model.versions.firstIndex(where: \.installed) ?? 0
                }

                Toggle("Include previews of upcoming Android releases", isOn: $model.includePreviews)

                // A Googlebook device has one size, a 1920 by 1200 laptop screen.
                if !isGooglebook {
                    // A Wear OS or TV version always makes a watch or a TV.
                    if let form = chosenForm {
                        Text("Size: \(form)")
                    } else {
                        Picker("Size", selection: $profile) {
                            ForEach(profiles.indices, id: \.self) { index in
                                Text(tr(profiles[index].0)).tag(index)
                            }
                        }
                        .onChange(of: profile) { _ in loadDefaults() }
                    }
                    TextField("Memory, in megabytes", text: $memory, prompt: Text(defaults.memory))
                        .accessibilityLabel("Memory, in megabytes")
                        .accessibilityHint("Default \(defaults.memory)")
                    TextField("Processor cores", text: $cores, prompt: Text(defaults.cores))
                        .accessibilityLabel("Processor cores")
                        .accessibilityHint("Default \(defaults.cores)")
                    TextField("Storage, in megabytes", text: $storage, prompt: Text(defaults.storage))
                        .accessibilityLabel("Storage, in megabytes")
                        .accessibilityHint("Default \(defaults.storage)")
                    Text("Empty fields use the defaults shown.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }

                HStack {
                    Text(tr(screenReaderText))
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
            loadDefaults()
        }
    }

    /// AAE's memory, cores and storage for the size and version chosen, shown in empty fields.
    private func loadDefaults() {
        let tag = model.versions.indices.contains(versionIndex) ? model.versions[versionIndex].tag : ""
        defaults = model.defaultHardware(for: profiles[profile].1, tag: tag)
    }

    private var isGooglebook: Bool {
        model.versions.indices.contains(versionIndex) && model.versions[versionIndex].tag == "googlebook"
    }

    /// "Watch" or "TV" for a Wear OS or TV version, which decides the size.
    private var chosenForm: String? {
        guard model.versions.indices.contains(versionIndex) else { return nil }
        let form = model.versions[versionIndex].form
        return form.isEmpty ? nil : form
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
        panel.message = tr("Choose a screen reader APK, such as a Backtalk build.")
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data]
        if panel.runModal() == .OK, let url = panel.url {
            screenReader = url.path
        }
    }

    private func create() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, model.versions.indices.contains(versionIndex) else { return }
        var hardware = HardwareChoice(memoryMb: nil, cores: nil, storageMb: nil)
        if !isGooglebook {
            let fields = [("Memory", memory), ("Processor cores", cores), ("Storage", storage)]
            var numbers: [UInt32?] = []
            for (label, text) in fields {
                let text = text.trimmingCharacters(in: .whitespaces)
                if text.isEmpty {
                    numbers.append(nil)
                } else if let number = UInt32(text) {
                    numbers.append(number)
                } else {
                    model.announce("\(label) takes a whole number.", tone: .failure)
                    return
                }
            }
            hardware = HardwareChoice(memoryMb: numbers[0], cores: numbers[1], storageMb: numbers[2])
            // Before anything downloads.
            guard model.checkHardware(hardware) else { return }
        }
        dismiss()
        model.create(
            name: trimmed,
            version: model.versions[versionIndex],
            profile: profiles[profile].1,
            hardware: hardware,
            screenReader: screenReader,
            volumeBoost: volumeBoost
        )
    }
}
