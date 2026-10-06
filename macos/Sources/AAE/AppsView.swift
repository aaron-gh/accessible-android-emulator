import SwiftUI

/// The selected device's installed apps, by name, and what can be done with
/// each: open, stop, clear, uninstall, and permissions and special access.
struct AppsView: View {
    @EnvironmentObject var model: AppModel
    @State private var selection: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.appsDevice.map { "Apps on \($0)" } ?? "Apps")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Toggle("Show Android's own apps", isOn: $model.showSystemApps)
            List(model.apps, id: \.package, selection: $selection) { app in
                Text(row(app))
            }
            .accessibilityLabel("Apps")
            .frame(minHeight: 260)
            .overlay {
                if model.apps.isEmpty {
                    Text(emptyText).foregroundStyle(.secondary)
                }
            }
            HStack {
                Button("Open") { selected.map(model.openApp) }
                    .keyboardShortcut("o")
                    .disabled(selected?.launchable != true)
                Button("Force Stop") { selected.map(model.forceStopApp) }
                Button("Permissions…") { selected.map(model.showPermissions) }
                    .keyboardShortcut("p")
                Spacer()
                Button("Clear Data…") { selected.map(model.clearAppData) }
                Button("Uninstall…") { selected.map(model.uninstallApp) }
                    .disabled(selected?.system == true)
            }
            .disabled(selected == nil)
            Button("Refresh") { model.loadApps() }
                .keyboardShortcut("r")
        }
        .padding()
        .frame(minWidth: 580, minHeight: 420)
        .onAppear { model.loadApps() }
        .onChange(of: model.selection) { _ in model.loadApps() }
        .sheet(isPresented: Binding(
            get: { model.permissionsShown != nil },
            set: { if !$0 { model.permissionsShown = nil } }
        )) {
            if let shown = model.permissionsShown {
                PermissionsView(app: shown.app, permissions: shown.permissions)
            }
        }
    }

    private var selected: AppInfo? {
        model.apps.first { $0.package == selection }
    }

    private var emptyText: String {
        if !(model.selected?.running ?? false) { return "Start the device to see its apps." }
        return model.loadingApps ? "Reading…" : "No apps you've installed. Show Android's own apps to see the rest."
    }

    private func row(_ app: AppInfo) -> String {
        var parts = [app.label, app.package]
        if !app.version.isEmpty { parts.append("version \(app.version)") }
        if !app.enabled { parts.append("turned off") }
        return parts.joined(separator: ", ")
    }
}

/// An app's permissions and special access, each a switch.
struct PermissionsView: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let app: AppInfo
    let permissions: AppPermissions

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Permissions of \(app.label)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    if permissions.permissions.isEmpty {
                        Text("It asks for no permissions.")
                    }
                    ForEach(permissions.permissions, id: \.name) { permission in
                        Toggle(permission.label, isOn: Binding(
                            get: { permission.granted },
                            set: { model.setPermission(app, permission, granted: $0) }
                        ))
                    }
                    Text("Special access")
                        .font(.headline)
                        .accessibilityAddTraits(.isHeader)
                        .padding(.top, 8)
                    ForEach(permissions.access, id: \.name) { access in
                        Toggle(access.name, isOn: Binding(
                            get: { access.allowed },
                            set: { model.setAccess(app, access, allowed: $0) }
                        ))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 240)
            HStack {
                Button("Grant All") { model.grantAllPermissions(app) }
                    .disabled(permissions.permissions.allSatisfy(\.granted))
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding()
        .frame(width: 480)
    }
}

/// The Device menu item that opens the Apps window.
struct AppsMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Apps") { openWindow(id: "apps") }
            .keyboardShortcut("p", modifiers: [.command, .option])
    }
}
