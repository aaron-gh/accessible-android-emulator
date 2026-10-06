import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The device log, one line per row, filtered by app, tag, level and text.
/// New lines appear as they're written, while the window is open.
struct LogView: View {
    @EnvironmentObject var model: AppModel
    @State private var selection: UInt64?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.logDevice.map { "Log of \($0)" } ?? "Device Log")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)

            HStack {
                Picker("App", selection: $model.logApp) {
                    Text("All apps").tag("")
                    ForEach(apps, id: \.self) { Text($0).tag($0) }
                }
                .frame(maxWidth: 320)
                Picker("Level", selection: $model.logLevel) {
                    Text("All levels").tag(LogLevel?.none)
                    Text("Debug and above").tag(LogLevel?.some(.debug))
                    Text("Info and above").tag(LogLevel?.some(.info))
                    Text("Warnings and errors").tag(LogLevel?.some(.warning))
                    Text("Errors").tag(LogLevel?.some(.error))
                    Text("Fatal errors").tag(LogLevel?.some(.fatal))
                }
                .frame(maxWidth: 260)
            }
            HStack {
                TextField("Tag", text: $model.logTag)
                    .frame(maxWidth: 200)
                TextField("Search", text: $model.logSearch)
            }
            HStack {
                Toggle("Pause", isOn: $model.logPaused)
                Toggle("Announce new errors in this list", isOn: $model.announceLogErrors)
                Spacer()
                Text(count)
                    .foregroundStyle(.secondary)
            }
            if let problem = model.logProblem {
                Text(problem)
                    .foregroundStyle(.red)
            }

            List(model.logEntries, id: \.seq, selection: $selection) { entry in
                Text("\(entry.clock)  \(entry.spoken)")
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(color(entry.level))
                    .lineLimit(3)
                    .accessibilityLabel("\(entry.spoken). \(entry.process ?? "Unknown process"), at \(entry.clock)")
            }
            .accessibilityLabel("Log lines")
            .frame(minHeight: 300)

            HStack {
                Button("Copy Line") { copy(selectedLines) }
                    .disabled(selection == nil)
                Button("Copy All Shown") { copy(allLines) }
                    .disabled(model.logEntries.isEmpty)
                Button("Save…") { save() }
                    .disabled(model.logEntries.isEmpty)
                Spacer()
                Button("Clear") { model.clearLogs() }
                    .disabled(model.logEntries.isEmpty)
            }
        }
        .padding()
        .frame(minWidth: 680, minHeight: 500)
        .onAppear { model.watchLogs() }
        .onDisappear { model.stopWatchingLogs() }
        .onChange(of: model.selection) { _ in model.watchLogs() }
    }

    /// The processes to choose from, keeping the chosen one even before it logs.
    private var apps: [String] {
        let chosen = model.logApp
        if chosen.isEmpty || model.logProcesses.contains(chosen) {
            return model.logProcesses
        }
        return [chosen] + model.logProcesses
    }

    private var count: String {
        let n = model.logEntries.count
        return n == 1 ? "1 line" : "\(n) lines"
    }

    private func color(_ level: LogLevel) -> Color {
        switch level {
        case .error, .fatal: .red
        case .warning: .orange
        case .verbose, .debug: .secondary
        case .info: .primary
        }
    }

    private var selectedLines: String {
        model.logEntries.first { $0.seq == selection }.map { $0.line + "\n" } ?? ""
    }

    private var allLines: String {
        model.logEntries.map(\.line).joined(separator: "\n") + "\n"
    }

    private func copy(_ text: String) {
        guard !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        model.announce("Copied.")
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(model.logDevice ?? "device") log.txt"
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try allLines.write(to: url, atomically: true, encoding: .utf8)
            model.announce("Saved \(url.lastPathComponent).", tone: .success)
        } catch {
            model.announce(error.localizedDescription, tone: .failure)
        }
    }
}

/// The Device menu item that opens the device log window.
struct LogMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Device Log") { openWindow(id: "devicelog") }
            .keyboardShortcut("j", modifiers: [.command, .option])
    }
}
