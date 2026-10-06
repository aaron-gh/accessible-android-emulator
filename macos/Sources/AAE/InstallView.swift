import SwiftUI

/// Apps to install, waiting for the user to choose which running devices.
struct InstallQuestion: Identifiable {
    let id = UUID()
    let paths: [String]
    let devices: [DeviceInfo]
    let chosen: Set<String>
}

/// Asks which running devices to install apps on.
struct InstallView: View {
    @EnvironmentObject var model: AppModel
    let question: InstallQuestion
    @State private var chosen: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Install \(names)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text("Which devices should it go on? Only running devices are listed.")
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
                Button("Install") {
                    // In the list's order, so installs happen in a predictable order.
                    model.install(question.paths, on: question.devices.map(\.id).filter(chosen.contains))
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
