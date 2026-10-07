import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// What the screen reader has said, with times, while the speech log is on.
/// New speech appears as it happens, while the window is open.
struct SpeechLogView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(model.speechLogDevice.map { "What \($0) said" } ?? "Speech Log")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                Toggle("Record speech", isOn: Binding(
                    get: { model.speechLogOn },
                    set: { model.setSpeechLog($0) }
                ))
                .disabled(model.speechLogBusy || model.speechLogDevice == nil)
            }

            List(Array(model.speechLog.enumerated()), id: \.offset) { _, utterance in
                Text("\(utterance.clock)  \(utterance.text)")
                    .accessibilityLabel("\(utterance.text), at \(utterance.clock)")
                    .textSelection(.enabled)
            }
            .accessibilityLabel("Speech")
            .frame(minHeight: 300)

            HStack {
                Button("Copy All") { copy() }
                    .disabled(model.speechLog.isEmpty)
                Button("Save…") { save() }
                    .disabled(model.speechLog.isEmpty)
                Spacer()
                Button("Clear") { model.clearSpeechLog() }
                    .disabled(model.speechLog.isEmpty)
            }
        }
        .padding()
        .frame(minWidth: 560, minHeight: 440)
        .onAppear { model.watchSpeechLog() }
        .onDisappear { model.stopWatchingSpeechLog() }
        .onChange(of: model.selection) { _ in model.watchSpeechLog() }
    }

    private var text: String {
        model.speechLog.map { "\($0.clock)  \($0.text)" }.joined(separator: "\n") + "\n"
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        model.announce("Copied the speech log.")
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(model.speechLogDevice ?? "device") speech.txt"
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try text.write(to: url, atomically: true, encoding: .utf8)
            model.announce("Saved \(url.lastPathComponent).", tone: .success)
        } catch {
            model.announce(error.localizedDescription, tone: .failure)
        }
    }
}

/// The Device menu item that opens the speech log window.
struct SpeechLogMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Speech Log") { openWindow(id: "speechlog") }
            .keyboardShortcut("l", modifiers: [.command, .option])
    }
}
