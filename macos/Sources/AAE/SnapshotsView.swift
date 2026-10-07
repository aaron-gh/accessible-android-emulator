import SwiftUI

/// The selected device's snapshots: saved states it can go back to, each with
/// a name, when it was taken, and notes.
struct SnapshotsView: View {
    @EnvironmentObject var model: AppModel
    @State private var selection: String?
    @State private var editing: SnapshotEdit?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.snapshotsDevice.map { "Snapshots of \($0)" } ?? "Snapshots")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)

            List(model.snapshots, id: \.id, selection: $selection) { snapshot in
                VStack(alignment: .leading, spacing: 2) {
                    Text(snapshot.name)
                    Text(details(snapshot))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    if !snapshot.notes.isEmpty {
                        Text(snapshot.notes)
                            .font(.callout)
                    }
                }
                .accessibilityElement(children: .combine)
            }
            .accessibilityLabel("Snapshots")
            .frame(minHeight: 220)
            .overlay {
                if model.snapshots.isEmpty {
                    Text(emptyText)
                        .foregroundStyle(.secondary)
                }
            }

            HStack {
                Button("Save Snapshot…") { editing = SnapshotEdit(snapshot: nil) }
                    .keyboardShortcut("s")
                Button("Restore…") { selected.map(model.restoreSnapshot) }
                    .disabled(selected?.compatible != true)
                Button("Edit…") { editing = SnapshotEdit(snapshot: selected) }
                    .disabled(selected == nil)
                Spacer()
                Button("Delete…") { selected.map(model.deleteSnapshot) }
                    .disabled(selected == nil)
            }
            .disabled(!(model.selected?.running ?? false))
        }
        .padding()
        .frame(minWidth: 560, minHeight: 400)
        .onAppear { model.loadSnapshots() }
        .onChange(of: model.selection) { _ in model.loadSnapshots() }
        .sheet(item: $editing) { edit in
            SnapshotEditor(edit: edit)
        }
    }

    private var selected: SnapshotInfo? {
        model.snapshots.first { $0.id == selection }
    }

    private var emptyText: String {
        if !(model.selected?.running ?? false) {
            return "Start the device to see its snapshots."
        }
        return model.loadingSnapshots ? "Reading…" : "No snapshots yet."
    }

    private func details(_ snapshot: SnapshotInfo) -> String {
        var parts: [String] = []
        if let taken = snapshot.taken { parts.append("Taken \(taken)") }
        parts.append(snapshot.size)
        if snapshot.loaded { parts.append("restored last") }
        if !snapshot.compatible { parts.append("this emulator can't restore it") }
        return parts.joined(separator: ", ") + "."
    }
}

/// A snapshot being saved (no snapshot) or edited.
struct SnapshotEdit: Identifiable {
    let id = UUID()
    let snapshot: SnapshotInfo?
}

/// Asks for a snapshot's name and notes.
struct SnapshotEditor: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let edit: SnapshotEdit
    @State private var name = ""
    @State private var notes = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(edit.snapshot == nil ? "Save a Snapshot" : "Edit \(edit.snapshot?.name ?? "")")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            TextField("Name", text: $name)
            Text("Notes")
            TextEditor(text: $notes)
                .accessibilityLabel("Notes")
                .frame(minHeight: 80)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(edit.snapshot == nil ? "Save" : "Save Changes") {
                    if let snapshot = edit.snapshot {
                        model.updateSnapshot(snapshot, name: name, notes: notes)
                    } else {
                        model.saveSnapshot(name: name, notes: notes)
                    }
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding()
        .frame(width: 420)
        .onAppear {
            name = edit.snapshot?.name ?? ""
            notes = edit.snapshot?.notes ?? ""
        }
    }
}

/// The Device menu item that opens the Snapshots window.
struct SnapshotsMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Snapshots") { openWindow(id: "snapshots") }
            .keyboardShortcut("s", modifiers: [.command, .option])
    }
}
