import SwiftUI

/// Apps to install, waiting for the user to choose which running devices.
struct InstallQuestion: Identifiable {
    let id = UUID()
    let paths: [String]
    let devices: [DeviceInfo]
    let chosen: Set<String>
    /// Watch the path for new builds, instead of installing once.
    var watch = false
}

/// Asks which running devices to install apps on.
struct InstallView: View {
    @EnvironmentObject var model: AppModel
    let question: InstallQuestion
    @State private var chosen: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(question.watch ? "Watch \(names) for new builds" : "Install \(names)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text(question.watch
                ? "Which devices should each new build go on? Only running devices are listed."
                : "Which devices should it go on? Only running devices are listed.")
            ForEach(question.devices, id: \.id) { device in
                Toggle("\(device.name), \(device.android)", isOn: Binding(
                    get: { chosen.contains(device.id) },
                    set: { if $0 { chosen.insert(device.id) } else { chosen.remove(device.id) } }
                ))
            }
            HStack {
                Button("Choose All") { chosen = Set(question.devices.map(\.id)) }
                Spacer()
                Button("Cancel", role: .cancel) { model.installQuestion = nil }
                    .keyboardShortcut(.cancelAction)
                Button(question.watch ? "Watch" : "Install") {
                    // In the list's order, so installs happen in a predictable order.
                    let ids = question.devices.map(\.id).filter(chosen.contains)
                    if question.watch {
                        model.startWatching(question.paths[0], on: ids)
                    } else {
                        model.install(question.paths, on: ids)
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(chosen.isEmpty)
            }
        }
        .padding()
        .frame(width: 460)
        .onAppear { chosen = question.chosen }
    }

    private var names: String {
        let files = question.paths.map { ($0 as NSString).lastPathComponent }
        return files.count == 1 ? files[0] : "\(files.count) apps"
    }
}
