import SwiftUI

/// A screen reader build waiting for the user to choose which devices get it.
struct ScreenReaderBuildQuestion: Identifiable {
    let id = UUID()
    let path: String
    let package: String
    let chosen: Set<String>
}

/// Asks which devices to install a screen reader build on. The devices that
/// already use that screen reader start chosen; a new build of it keeps its
/// settings. Stopped devices get it when they next start.
struct ScreenReaderBuildView: View {
    @EnvironmentObject var model: AppModel
    let question: ScreenReaderBuildQuestion
    @State private var chosen: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Install \(question.package)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text("Which devices should get this build? A new build of a device's screen reader keeps its settings. Stopped devices get it when they next start.")
            ForEach(model.devices, id: \.id) { device in
                Toggle(label(device), isOn: Binding(
                    get: { chosen.contains(device.id) },
                    set: { if $0 { chosen.insert(device.id) } else { chosen.remove(device.id) } }
                ))
            }
            HStack {
                Button("Choose All") { chosen = Set(model.devices.map(\.id)) }
                Spacer()
                Button("Cancel", role: .cancel) { model.screenReaderBuildQuestion = nil }
                    .keyboardShortcut(.cancelAction)
                Button("Install") {
                    model.installScreenReaderBuild(question.path, on: model.devices.map(\.id).filter(chosen.contains))
                }
                .keyboardShortcut(.defaultAction)
                .disabled(chosen.isEmpty)
            }
        }
        .padding()
        .frame(width: 520)
        .onAppear { chosen = question.chosen }
    }

    private func label(_ device: DeviceInfo) -> String {
        var parts = ["\(device.name), \(device.android)"]
        if let reader = device.screenReader {
            parts.append(reader == question.package ? "uses this screen reader" : "uses \(reader)")
        }
        if !device.running {
            parts.append("stopped")
        }
        return parts.joined(separator: ", ")
    }
}
