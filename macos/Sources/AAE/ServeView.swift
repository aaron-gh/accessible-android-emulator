import SwiftUI

/// A phone paired with this Mac's AAE.
struct PairedPhone: Identifiable, Decodable {
    let id: String
    let name: String
}

/// Serves this Mac's devices to AAE Remote, the Android app, by running the
/// bundled `aae serve` in the background. Its sound and vibrations go to the
/// phone attached to a device.
@MainActor
final class Serving: ObservableObject {
    @Published private(set) var on = false
    @Published private(set) var code: String?
    @Published private(set) var status = "Not serving."
    @Published private(set) var phones: [PairedPhone] = []
    /// Says things through VoiceOver, set by the app.
    var announce: @MainActor (String) -> Void = { _ in }

    private var process: Process?
    private var input: FileHandle?
    private var buffer = Data()

    /// The aae command inside the app.
    private var aae: URL? {
        let url = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/aae")
        return FileManager.default.isExecutableFile(atPath: url.path) ? url : nil
    }

    func start() {
        guard process == nil else { return }
        guard let aae else {
            status = "This copy of AAE doesn't include the aae command, which serves devices. Download AAE again."
            announce(status)
            return
        }
        let process = Process()
        process.executableURL = aae
        process.arguments = ["serve", "--json"]
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = FileHandle.nullDevice
        stdout.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            Task { @MainActor in self?.received(data) }
        }
        process.terminationHandler = { [weak self] ended in
            Task { @MainActor in
                guard let self, self.process === ended else { return }
                self.process = nil
                self.input = nil
                self.on = false
                self.code = nil
                self.status = ended.terminationStatus == 0 ? "Not serving." : "Serving stopped unexpectedly."
            }
        }
        do {
            try process.run()
            self.process = process
            input = stdin.fileHandleForWriting
            on = true
            status = "Starting."
        } catch {
            status = "Couldn't start serving: \(error.localizedDescription)"
            announce(status)
        }
    }

    func stop() {
        // Closing its input stops it.
        try? input?.close()
        input = nil
        process = nil
        on = false
        code = nil
        status = "Not serving."
        announce("Stopped serving.")
    }

    func newCode() {
        input?.write("p\n".data(using: .utf8)!)
    }

    private func received(_ data: Data) {
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            guard let json = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
            switch json["event"] as? String {
            case "serving":
                status = "Serving to AAE Remote on port \(json["port"] ?? "").\u{20}"
            case "code":
                let code = json["code"] as? String ?? ""
                self.code = code
                let minutes = json["minutes"] as? Int ?? 10
                announce("Pairing code: \(Self.spoken(code)). It works once, for \(minutes) minutes.")
            case "notice":
                let text = json["text"] as? String ?? ""
                status = text
                announce(text)
                loadPhones()
            default:
                break
            }
        }
    }

    /// A code read a character at a time, in its groups: "A B C D, E F G H".
    static func spoken(_ code: String) -> String {
        code.split(separator: "-").map { $0.map(String.init).joined(separator: " ") }.joined(separator: ", ")
    }

    func loadPhones() {
        guard let aae else { return }
        Task.detached {
            let process = Process()
            process.executableURL = aae
            process.arguments = ["phones", "--json"]
            let out = Pipe()
            process.standardOutput = out
            try? process.run()
            let data = out.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            let phones = (try? JSONDecoder().decode([PairedPhone].self, from: data)) ?? []
            await MainActor.run { self.phones = phones }
        }
    }

    func unpair(_ phone: PairedPhone) {
        guard let aae else { return }
        let process = Process()
        process.executableURL = aae
        process.arguments = ["phones", "--unpair", phone.id]
        try? process.run()
        process.waitUntilExit()
        announce("Unpaired \(phone.name).")
        loadPhones()
    }
}

/// The window for serving this Mac's devices to phones.
struct ServeView: View {
    @EnvironmentObject var serving: Serving

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Serve Devices to Phones")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            Text("AAE Remote, the Android app, uses this Mac's devices: their sound and vibrations play on the phone, and it sends them keys and touches. Phones on this network find the Mac; each pairs once with a code.")
                .foregroundStyle(.secondary)
            Toggle("Serve this Mac's devices to AAE Remote", isOn: Binding(
                get: { serving.on },
                set: { $0 ? serving.start() : serving.stop() }
            ))
            Text(serving.status)
            if let code = serving.code {
                HStack {
                    Text("Pairing code:")
                    Text(code)
                        .font(.system(.title2, design: .monospaced))
                        .textSelection(.enabled)
                        .accessibilityLabel(Serving.spoken(code))
                }
                Button("New Pairing Code") { serving.newCode() }
            }
            Text("Paired Phones")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            if serving.phones.isEmpty {
                Text("None yet.")
            }
            ForEach(serving.phones) { phone in
                HStack {
                    Text(phone.name)
                    Spacer()
                    Button("Unpair") { serving.unpair(phone) }
                        .accessibilityLabel("Unpair \(phone.name)")
                }
            }
            Spacer()
        }
        .padding()
        .frame(minWidth: 480, minHeight: 320)
        .onAppear { serving.loadPhones() }
    }
}

/// The File menu item that opens the serving window.
struct ServeMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Serve Devices to Phones…") { openWindow(id: "serving") }
    }
}
