import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// One window or element of the inspected screen.
struct InspectorItem: Identifiable, Hashable {
    let id: UInt32
    let depth: Int
    let summary: String
    let details: [String]
    var children: [InspectorItem]?
}

/// The accessibility inspector: the screen's accessibility tree as a screen
/// reader sees it, the details of the selected element, and the problems
/// found. Read it as a tree, or as a flat list in reading order with each
/// element's level, which needs no expanding.
struct InspectorView: View {
    @EnvironmentObject var model: AppModel
    @State private var selection: UInt32?
    @State private var flat = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(model.inspectedDevice.map { "Accessibility of \($0)" } ?? "Accessibility Inspector")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                Button("Refresh") { model.inspect() }
                    .keyboardShortcut("r")
                    .disabled(model.inspecting)
                Toggle("Follow the screen", isOn: Binding(
                    get: { model.followingScreen },
                    set: { model.followScreen($0) }
                ))
                Button("Copy as Text") { copyText() }
                    .disabled(model.inspection == nil)
                Button("Save…") { save() }
                    .disabled(model.inspection == nil)
            }

            if model.inspecting {
                ProgressView("Reading the screen").controlSize(.small)
            }

            if let inspection = model.inspection {
                let tree = buildTree(inspection.rows)
                Picker("View", selection: $flat) {
                    Text("Tree").tag(false)
                    Text("Flat list").tag(true)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 260)

                Text("Screen").font(.subheadline).bold().accessibilityAddTraits(.isHeader)
                Group {
                    if flat {
                        List(flatten(tree), selection: $selection) { item in
                            Text(String(repeating: "    ", count: item.depth) + item.summary)
                                .accessibilityLabel("Level \(item.depth + 1), \(item.summary)")
                                .tag(item.id)
                        }
                    } else {
                        List(tree, children: \.children, selection: $selection) { item in
                            Text(item.summary).tag(item.id)
                        }
                    }
                }
                .accessibilityLabel("Screen elements")
                .frame(minHeight: 260)

                Text("Details").font(.subheadline).bold().accessibilityAddTraits(.isHeader)
                List(selectedDetails(inspection), id: \.self) { line in
                    Text(line).textSelection(.enabled)
                }
                .accessibilityLabel("Details of the selected element")
                .frame(minHeight: 120)

                Text(inspection.issues.isEmpty ? "No problems found" : "Problems: \(inspection.issues.count)")
                    .font(.subheadline).bold()
                    .accessibilityAddTraits(.isHeader)
                if !inspection.issues.isEmpty {
                    List(Array(inspection.issues.enumerated()), id: \.offset) { _, issue in
                        Text("\(issue.error ? "Error" : "Warning"): \(issue.message) Element: \(issue.element)\(issue.id.map { " (\($0))" } ?? "")")
                            .textSelection(.enabled)
                    }
                    .accessibilityLabel("Problems")
                    .frame(minHeight: 100)
                }
            } else if !model.inspecting {
                Text("Refresh (Command-R) reads the screen.")
            }
        }
        .padding()
        .frame(minWidth: 620, minHeight: 560)
        .onAppear {
            if model.inspection == nil { model.inspect() }
        }
        .onDisappear { model.followScreen(false) }
    }

    private func buildTree(_ rows: [InspectorRow]) -> [InspectorItem] {
        var children: [UInt32?: [InspectorRow]] = [:]
        for row in rows {
            children[row.parent, default: []].append(row)
        }
        func build(_ row: InspectorRow, depth: Int) -> InspectorItem {
            let kids = (children[row.index] ?? []).map { build($0, depth: depth + 1) }
            return InspectorItem(
                id: row.index, depth: depth, summary: row.summary, details: row.details,
                children: kids.isEmpty ? nil : kids
            )
        }
        return (children[nil] ?? []).map { build($0, depth: 0) }
    }

    private func flatten(_ items: [InspectorItem]) -> [InspectorItem] {
        items.flatMap { [$0] + flatten($0.children ?? []) }
    }

    private func selectedDetails(_ inspection: Inspection) -> [String] {
        guard let selection, let row = inspection.rows.first(where: { $0.index == selection }) else {
            return ["Select an element to see its details."]
        }
        return [row.summary] + row.details
    }

    private func copyText() {
        guard let text = model.inspection?.text else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        model.announce("Copied the screen's elements as text.")
    }

    private func save() {
        guard let inspection = model.inspection else { return }
        let panel = NSSavePanel()
        panel.message = "Save as a web page, with the problems found; as JSON, with every property; or as text. The name's ending chooses."
        panel.nameFieldStringValue = "\(model.inspectedDevice ?? "screen") accessibility.html"
        panel.allowedContentTypes = [.html, .json, .plainText]
        panel.allowsOtherFileTypes = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let contents: String
        switch url.pathExtension.lowercased() {
        case "html", "htm": contents = inspection.html
        case "json": contents = inspection.json
        default: contents = inspection.text
        }
        do {
            try contents.write(to: url, atomically: true, encoding: .utf8)
            model.announce("Saved \(url.lastPathComponent).", tone: .success)
        } catch {
            model.announce(error.localizedDescription, tone: .failure)
        }
    }
}

/// The Device menu item that opens the inspector window.
struct InspectorMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Accessibility Inspector") { openWindow(id: "inspector") }
            .keyboardShortcut("i", modifiers: [.command, .option])
    }
}
