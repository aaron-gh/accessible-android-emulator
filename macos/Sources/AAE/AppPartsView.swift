import SwiftUI

/// A question about an installed app's special parts: accessibility
/// services, keyboards, notification listeners and device administrators.
struct PartsQuestion: Identifiable {
    let id = UUID()
    let package: String
    let parts: [AppPartInfo]
    let answer: ([AppChoice]) -> Void
}

/// Asks which of an app's special parts to turn on. Accessibility services
/// start on, the rest off. The answers are remembered for this device and
/// used again when the app is reinstalled.
struct AppPartsView: View {
    let question: PartsQuestion
    @State private var on: [String: Bool] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Turn on parts of \(question.package)?")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            ForEach(question.parts, id: \.component) { part in
                Toggle("\(part.name), \(part.kindDescription)", isOn: binding(part))
            }
            HStack {
                Spacer()
                Button("Leave All Off", role: .cancel) {
                    question.answer(question.parts.map { AppChoice(part: $0, on: false) })
                }
                .keyboardShortcut(.cancelAction)
                Button("Done") {
                    question.answer(question.parts.map { AppChoice(part: $0, on: on[$0.component] ?? false) })
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding()
        .frame(width: 480)
        .onAppear {
            for part in question.parts {
                on[part.component] = part.kind == .accessibilityService
            }
        }
    }

    private func binding(_ part: AppPartInfo) -> Binding<Bool> {
        Binding(
            get: { on[part.component] ?? false },
            set: { on[part.component] = $0 }
        )
    }
}
