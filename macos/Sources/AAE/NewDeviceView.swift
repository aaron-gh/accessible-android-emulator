import SwiftUI
import UniformTypeIdentifiers

/// Creates a device: a name, an Android version, and how to set it up.
struct NewDeviceView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var imageIndex = 0
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

            if model.images.isEmpty {
                Text("No Android versions are installed that run on this Mac. Install a system image with Android Studio's SDK Manager.")
            } else {
                TextField("Name", text: $name)

                Picker("Android version", selection: $imageIndex) {
                    ForEach(model.images.indices, id: \.self) { index in
                        Text(model.images[index].description).tag(index)
                    }
                }

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
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || model.images.isEmpty)
            }
        }
        .padding()
        .frame(width: 480)
        .onAppear { screenReader = model.defaultScreenReader }
    }

    private var screenReaderText: String {
        guard let screenReader else { return "TalkBack, if this Android version has it" }
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
        guard !trimmed.isEmpty, model.images.indices.contains(imageIndex) else { return }
        dismiss()
        model.create(
            name: trimmed,
            image: model.images[imageIndex],
            profile: profiles[profile].1,
            screenReader: screenReader,
            volumeBoost: volumeBoost
        )
    }
}
