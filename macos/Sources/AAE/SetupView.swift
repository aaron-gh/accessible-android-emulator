import SwiftUI

/// Shown until the Android emulator and SDK tools AAE needs are installed:
/// what will be downloaded and where, any reason this Mac can't run the
/// emulator, and the button that starts the download.
struct SetupView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Set up AAE")
                .font(.title2)
                .accessibilityAddTraits(.isHeader)
            if let status = model.setupStatus {
                Text("AAE needs Google's Android emulator and tools. It downloads them once, \(status.missingSize) in all, and checks each download.")
                ForEach(status.missing, id: \.name) { tool in
                    Text("\(tool.name) \(tool.revision), \(tool.size)")
                }
                Text(status.ownSdk
                    ? "They go in AAE's own folder, \(status.sdkPath)."
                    : "They go in the Android SDK at \(status.sdkPath), which Android Studio uses too.")
                    .foregroundStyle(.secondary)
                if let problem = status.virtualisationProblem {
                    Text(problem)
                        .foregroundStyle(.red)
                }
                if let warning = status.performanceWarning {
                    Text(warning)
                }
                if let download = model.download {
                    ProgressView(value: Double(download.percent), total: 100) {
                        Text("Downloading: \(download.percent)%")
                    }
                    Text(model.status)
                        .foregroundStyle(.secondary)
                } else {
                    Button("Download and Set Up") { model.runSetup() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(model.settingUp || status.virtualisationProblem != nil || status.missing.isEmpty)
                }
            } else if model.setupError == nil {
                Text("Checking what AAE needs…")
            }
            if let error = model.setupError, !model.settingUp {
                Text(error)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
                Button("Try Again") { model.checkSetup(refresh: true) }
            }
            Spacer()
        }
        .padding()
    }
}
