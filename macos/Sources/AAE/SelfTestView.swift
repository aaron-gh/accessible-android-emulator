import AppKit
import SwiftUI

/// One self-test check, as the window lists it.
struct SelfTestRow: Identifiable {
    let id = UUID()
    let name: String
    let outcome: CheckOutcome
    let detail: String

    var label: String {
        switch outcome {
        case .passed: "OK"
        case .warning: "Warning"
        case .failed: "Problem"
        }
    }
}

/// The self-test's results: problems first, then warnings, then the rest.
struct SelfTestView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Self-Test")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text(summary)
            List(sorted) { row in
                Text("\(row.label): \(row.name). \(row.detail)")
                    .foregroundStyle(row.outcome == .failed ? .red : row.outcome == .warning ? .orange : .primary)
            }
            .accessibilityLabel("Checks")
            .frame(minHeight: 260)
            HStack {
                Button("Run Again") { model.runSelfTest() }
                    .keyboardShortcut("r")
                    .disabled(model.selfTestRunning)
                Button("Copy") {
                    let text = sorted.map { "\($0.label): \($0.name). \($0.detail)" }.joined(separator: "\n")
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                    model.announce("Copied the results.")
                }
                .disabled(model.selfTestResults.isEmpty)
            }
        }
        .padding()
        .frame(minWidth: 560, minHeight: 400)
        .onAppear {
            if model.selfTestResults.isEmpty { model.runSelfTest() }
        }
    }

    private var sorted: [SelfTestRow] {
        let rank: (CheckOutcome) -> Int = { $0 == .failed ? 0 : $0 == .warning ? 1 : 2 }
        return model.selfTestResults.sorted { rank($0.outcome) < rank($1.outcome) }
    }

    private var summary: String {
        if model.selfTestRunning { return "Checking…" }
        let results = model.selfTestResults
        let problems = results.filter { $0.outcome == .failed }.count
        let warnings = results.filter { $0.outcome == .warning }.count
        return "\(results.count) checks: \(results.count - problems - warnings) passed, \(warnings) warnings, \(problems) problems."
    }
}

/// The Help menu item that opens the self-test and runs it.
struct SelfTestMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    let model: AppModel

    var body: some View {
        Button("Run Self-Test") {
            openWindow(id: "selftest")
            model.runSelfTest()
        }
    }
}
