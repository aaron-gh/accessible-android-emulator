import AppKit
import SwiftUI

/// Asks the user to accept Google's licence before a download. The text is in
/// a read-only text view, so VoiceOver reads it line by line and paragraph by
/// paragraph. Decline is the cancel action, so Escape declines; nothing is the
/// default action, so a stray Return accepts nothing.
struct LicenceView: View {
    let request: LicenceRequest

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Google's licence for \(request.version)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text("To download \(request.version), you need to accept Google's licence. It's below.")
            ReadOnlyText(text: request.licence.text, label: "Licence text")
                .frame(minHeight: 320)
            HStack {
                Spacer()
                Button("Decline", role: .cancel) { request.answer(false) }
                    .keyboardShortcut(.cancelAction)
                Button("Accept") { request.answer(true) }
            }
        }
        .padding()
        .frame(width: 620)
    }
}

/// A pending licence question.
struct LicenceRequest: Identifiable {
    let id = UUID()
    let version: String
    let licence: LicenceInfo
    let answer: (Bool) -> Void
}

/// A scrolling, read-only, selectable text view.
struct ReadOnlyText: NSViewRepresentable {
    let text: String
    let label: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        let view = scroll.documentView as! NSTextView
        view.isEditable = false
        view.isSelectable = true
        view.font = .systemFont(ofSize: NSFont.systemFontSize)
        view.textContainerInset = NSSize(width: 6, height: 6)
        view.string = text
        view.setAccessibilityLabel(label)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        if let view = scroll.documentView as? NSTextView, view.string != text {
            view.string = text
        }
    }
}
