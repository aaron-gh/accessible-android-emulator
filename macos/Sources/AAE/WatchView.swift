import SwiftUI

/// An app's build output being watched for new builds.
struct BuildWatch: Identifiable {
    let id = UUID()
    let path: String
    let deviceIDs: [String]
    var lastBuild: Date?
    var task: Task<Void, Never>?

    /// What identifies one build: the newest APK at the path, and when it
    /// changed and how big it is.
    struct Build: Equatable {
        let url: URL
        let modified: Date
        let size: Int
    }

    /// The newest APK at a path: the file itself, or the most recently changed
    /// one in a folder, a few levels down.
    static func currentBuild(at path: String) -> Build? {
        let url = URL(fileURLWithPath: path)
        let keys: [URLResourceKey] = [.contentModificationDateKey, .fileSizeKey, .isRegularFileKey]
        func build(_ url: URL) -> Build? {
            guard let values = try? url.resourceValues(forKeys: Set(keys)),
                  values.isRegularFile == true,
                  let modified = values.contentModificationDate else { return nil }
            return Build(url: url, modified: modified, size: values.fileSize ?? 0)
        }
        if url.pathExtension.lowercased() == "apk" {
            return build(url)
        }
        guard let walker = FileManager.default.enumerator(at: url, includingPropertiesForKeys: keys) else { return nil }
        var newest: Build?
        for case let file as URL in walker {
            if walker.level > 7 { walker.skipDescendants() }
            guard file.pathExtension.lowercased() == "apk", let found = build(file) else { continue }
            if newest.map({ found.modified > $0.modified }) ?? true { newest = found }
        }
        return newest
    }
}

/// The build outputs being watched, each with a Stop button.
struct WatchView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Watching for New Builds")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            if model.watches.isEmpty {
                Text("Nothing is being watched. Choose Watch for New Builds in the Device menu.")
            }
            ForEach(model.watches) { watch in
                HStack {
                    Text(describe(watch))
                    Spacer()
                    Button("Stop") { model.stopWatching(watch) }
                        .accessibilityLabel("Stop watching \((watch.path as NSString).lastPathComponent)")
                }
            }
            Spacer()
            Button("Watch Another…") { model.watchForBuilds() }
        }
        .padding()
        .frame(minWidth: 520, minHeight: 240)
    }

    private func describe(_ watch: BuildWatch) -> String {
        let names = watch.deviceIDs.compactMap { id in model.devices.first { $0.id == id }?.name }.joined(separator: ", ")
        var text = "\(watch.path), for \(names)."
        if let last = watch.lastBuild {
            text += " Last installed at \(last.formatted(date: .omitted, time: .shortened))."
        }
        return text
    }
}

/// The Device menu items for watching builds.
struct WatchMenuItems: View {
    @Environment(\.openWindow) private var openWindow
    let model: AppModel

    var body: some View {
        Button("Watch for New Builds…") {
            openWindow(id: "watches")
            model.watchForBuilds()
        }
    }
}
