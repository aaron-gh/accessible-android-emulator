import SwiftUI

/// Asks for a link to open on the selected device, and which app opens it.
struct OpenLinkView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var link = ""
    @State private var package = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Open a Link")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            TextField("Link, such as https://example.com or myapp://settings", text: $link)
            Picker("Open in", selection: $package) {
                Text("Whichever app Android chooses").tag("")
                ForEach(model.apps.filter(\.launchable), id: \.package) { app in
                    Text(app.label).tag(app.package)
                }
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Open") {
                    model.openLink(link, package: package.isEmpty ? nil : package)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(link.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding()
        .frame(width: 480)
        .onAppear {
            if model.apps.isEmpty { model.loadApps() }
        }
    }
}

/// Asks for an intent to send on the selected device, for testing how an app
/// answers it.
struct SendIntentView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var action = ""
    @State private var data = ""
    @State private var target = ""
    @State private var extras = ""
    @State private var broadcast = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Send an Intent")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            TextField("Action, such as android.intent.action.VIEW", text: $action)
            TextField("Data, such as a link", text: $data)
            TextField("To: a package, or package/class for a screen or receiver", text: $target)
            Text("Text extras, one key=value on each line")
            TextEditor(text: $extras)
                .font(.system(.body, design: .monospaced))
                .accessibilityLabel("Text extras, one key=value on each line")
                .frame(minHeight: 60)
            Toggle("Send as a broadcast, not to open a screen", isOn: $broadcast)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Send") {
                    let pairs = extras.split(separator: "\n").compactMap { line -> IntentExtra? in
                        let parts = line.split(separator: "=", maxSplits: 1)
                        guard parts.count == 2 else { return nil }
                        return IntentExtra(key: String(parts[0]).trimmingCharacters(in: .whitespaces), value: String(parts[1]))
                    }
                    model.sendIntent(IntentInfo(action: action, data: data, target: target, extras: pairs, broadcast: broadcast))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(action.isEmpty && data.isEmpty && target.isEmpty)
            }
        }
        .padding()
        .frame(width: 520)
    }
}
