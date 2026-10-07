import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Runs commands in the device's shell and shows what they printed, as a
/// transcript that can be read line by line.
struct ShellView: View {
    @EnvironmentObject var model: AppModel
    @State private var command = ""
    @FocusState private var commandFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.selected.map { "Shell of \($0.name)" } ?? "Shell")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            HStack {
                TextField("Command", text: $command)
                    .font(.system(.body, design: .monospaced))
                    .focused($commandFocused)
                    .onSubmit(run)
                Button(model.shellRunning ? "Running…" : "Run", action: run)
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.shellRunning || command.trimmingCharacters(in: .whitespaces).isEmpty)
                Menu("Recent") {
                    ForEach(model.shellHistory.reversed(), id: \.self) { recent in
                        Button(recent) {
                            command = recent
                            commandFocused = true
                        }
                    }
                }
                .frame(maxWidth: 110)
                .disabled(model.shellHistory.isEmpty)
            }
            Text("Runs as the shell user, with a two-minute limit.")
                .font(.callout)
                .foregroundStyle(.secondary)

            ReadOnlyText(text: model.shellTranscript, label: "Output", monospaced: true, followsEnd: true)
                .frame(minHeight: 300)

            HStack {
                Button("Copy All") { copy() }
                    .disabled(model.shellTranscript.isEmpty)
                Button("Save…") { save() }
                    .disabled(model.shellTranscript.isEmpty)
                Spacer()
                Button("Clear") { model.clearShell() }
                    .disabled(model.shellTranscript.isEmpty)
            }
        }
        .padding()
        .frame(minWidth: 620, minHeight: 480)
        .onAppear { commandFocused = true }
    }

    private func run() {
        guard !model.shellRunning else { return }
        model.runShellCommand(command)
        command = ""
        commandFocused = true
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(model.shellTranscript, forType: .string)
        model.announce("Copied the output.")
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(model.selected?.name ?? "device") shell.txt"
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try model.shellTranscript.write(to: url, atomically: true, encoding: .utf8)
            model.announce("Saved \(url.lastPathComponent).", tone: .success)
        } catch {
            model.announce(error.localizedDescription, tone: .failure)
        }
    }
}

/// The Device menu item that opens the shell window.
struct ShellMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Shell") { openWindow(id: "shell") }
            .keyboardShortcut("t", modifiers: [.command, .option])
    }
}
