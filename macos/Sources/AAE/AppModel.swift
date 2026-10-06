import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Passes progress sentences from the Rust core to the main thread.
final class ProgressRelay: ProgressListener, @unchecked Sendable {
    private let handler: @MainActor (String) -> Void

    init(_ handler: @escaping @MainActor (String) -> Void) {
        self.handler = handler
    }

    func progress(message: String) {
        Task { @MainActor in handler(message) }
    }
}

/// Everything the window shows and every action the user can take.
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var devices: [DeviceInfo] = []
    @Published private(set) var images: [ImageInfo] = []
    @Published var selection: String?
    /// What each busy device is doing, by device id.
    @Published private(set) var busy: [String: String] = [:]
    /// The latest announcement, also shown in the window.
    @Published private(set) var status = ""
    /// The device whose keyboard is captured, if any.
    @Published private(set) var deviceModeID: String?
    @Published var showingNewDevice = false
    @Published var renaming: DeviceInfo?
    @Published var cloning: DeviceInfo?

    let engine: Engine?
    let startupError: String?
    private var sessions: [String: Session] = [:]
    /// First-start choices for devices just created, by device id.
    private var firstStart: [String: (screenReader: String?, volumeBoost: Bool)] = [:]

    init() {
        do {
            engine = try Engine()
            startupError = nil
        } catch {
            engine = nil
            startupError = error.localizedDescription
        }
        refresh()
    }

    var selected: DeviceInfo? {
        devices.first { $0.id == selection }
    }

    var inDeviceMode: Bool { deviceModeID != nil }

    func refresh() {
        guard let engine else { return }
        do {
            devices = try engine.devices()
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
        images = engine.images()
        if selected == nil {
            selection = devices.first?.id
        }
    }

    func announce(_ text: String, tone: Tone = .info) {
        status = text
        tone.play()
        Announcer.shared.say(text, interrupt: tone == .failure)
    }

    var defaultScreenReader: String? { engine?.defaultScreenReader() }

    // MARK: - Devices

    func create(name: String, image: ImageInfo, profile: DeviceProfile, screenReader: String?, volumeBoost: Bool) {
        guard let engine else { return }
        do {
            let device = try engine.createDevice(name: name, sysdir: image.sysdir, profile: profile)
            firstStart[device.id] = (screenReader, volumeBoost)
            refresh()
            selection = device.id
            announce("Created \(device.name).", tone: .success)
            start(device.id)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    func start(_ id: String? = nil) {
        guard let engine, let id = id ?? selection, busy[id] == nil else { return }
        let choices = firstStart[id] ?? (defaultScreenReader, true)
        busy[id] = "Starting"
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                try await engine.startDevice(
                    id: id,
                    screenReaderApk: choices.screenReader,
                    volumeBoost: choices.volumeBoost,
                    listener: relay
                )
                firstStart[id] = nil
                _ = try await session(for: id)
                Tone.success.play()
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
        }
    }

    func stop(_ id: String? = nil) {
        guard let engine, let id = id ?? selection, busy[id] == nil else { return }
        if deviceModeID == id {
            leaveDeviceMode()
        }
        let name = devices.first { $0.id == id }?.name ?? "The device"
        busy[id] = "Stopping"
        announce("Stopping \(name).")
        sessions.removeValue(forKey: id)?.stopAudio()
        Task {
            do {
                try await engine.stopDevice(id: id)
                announce("\(name) is stopped.", tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
        }
    }

    func delete() {
        guard let engine, let device = selected else { return }
        if device.running {
            announce("Stop \(device.name) before deleting it.", tone: .failure)
            return
        }
        let size = (try? engine.deviceSize(id: device.id)) ?? "all"
        let alert = NSAlert()
        alert.messageText = "Delete \(device.name)?"
        alert.informativeText = "This deletes the device and its \(size) of files. It can't be undone."
        alert.alertStyle = .warning
        // Cancel is the default, so a stray Return deletes nothing.
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Delete")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        do {
            let freed = try engine.deleteDevice(id: device.id)
            selection = nil
            refresh()
            announce("Deleted \(device.name). Freed \(freed).", tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    func rename(_ device: DeviceInfo, to name: String) {
        guard let engine else { return }
        do {
            let renamed = try engine.renameDevice(id: device.id, newName: name)
            refresh()
            announce("Renamed \(device.name) to \(renamed.name).", tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    func clone(_ device: DeviceInfo, as name: String) {
        guard let engine else { return }
        announce("Copying \(device.name).")
        do {
            let copy = try engine.cloneDevice(id: device.id, newName: name)
            refresh()
            selection = copy.id
            announce("Created \(copy.name), a copy of \(device.name).", tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    // MARK: - Sessions

    private func session(for id: String) async throws -> Session {
        if let session = sessions[id] {
            return session
        }
        guard let engine else { throw AaeError.Failed(message: "AAE's core did not start.") }
        let session = try await engine.openSession(id: id)
        try await session.startAudio()
        sessions[id] = session
        return session
    }

    /// Runs an action on the selected device's session, announcing any failure.
    private func withSession(_ action: @escaping @MainActor (Session) async throws -> Void) {
        guard let device = selected else { return }
        guard device.running else {
            announce("\(device.name) is not running. Start it first.", tone: .failure)
            return
        }
        Task {
            do {
                try await action(try await session(for: device.id))
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    // MARK: - Device mode

    func enterDeviceMode() {
        guard let device = selected, !inDeviceMode else { return }
        guard device.running else {
            announce("\(device.name) is not running. Start it first.", tone: .failure)
            return
        }
        Task {
            do {
                let session = try await session(for: device.id)
                deviceModeID = device.id
                activeSession = session
                DeviceModeLock.shared.lock { [weak self] in self?.leaveDeviceMode() }
                announce("Android keyboard on. Control Command Escape returns to the Mac.")
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    func leaveDeviceMode() {
        guard inDeviceMode else { return }
        DeviceModeLock.shared.unlock()
        deviceModeID = nil
        activeSession = nil
        announce("Mac keyboard on.")
    }

    /// The session keys go to in device mode.
    private(set) var activeSession: Session?

    func sendKey(_ keycode: UInt16, _ down: Bool) {
        _ = activeSession?.macKey(keycode: keycode, down: down)
    }

    // MARK: - Device actions

    func press(_ key: String) {
        withSession { session in try await session.press(name: key) }
    }

    func showNotifications() {
        withSession { session in _ = try await session.shell(command: "cmd statusbar expand-notifications") }
    }

    func showQuickSettings() {
        withSession { session in _ = try await session.shell(command: "cmd statusbar expand-settings") }
    }

    func rotate(left: Bool) {
        withSession { [weak self] session in
            let orientation = try await session.rotate(left: left)
            self?.announce("\(orientation).")
        }
    }

    func toggleMute() {
        withSession { [weak self] session in
            let muted = session.toggleMute()
            self?.announce(muted ? "Device audio muted." : "Device audio on.")
        }
    }

    func speakStatus() {
        guard let device = selected else {
            announce("No device is selected.")
            return
        }
        var parts = ["\(device.name), \(device.android)."]
        if let doing = busy[device.id] {
            parts.append("\(doing).")
        } else {
            parts.append(device.running ? "Running." : "Stopped.")
        }
        parts.append(inDeviceMode ? "The keyboard is in Android." : "The keyboard is on the Mac.")
        guard device.running, busy[device.id] == nil else {
            announce(parts.joined(separator: " "))
            return
        }
        withSession { [weak self] session in
            parts.append(try await session.screenReaderStatus())
            self?.announce(parts.joined(separator: " "))
        }
    }

    func installApp() {
        let panel = NSOpenPanel()
        panel.message = "Choose an app to install."
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data]
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        let paths = panel.urls.map(\.path)
        withSession { [weak self] session in
            for path in paths {
                self?.announce("Installing \((path as NSString).lastPathComponent).")
                let result = try await session.installApk(path: path)
                self?.announce(result, tone: .success)
            }
        }
    }

    func screenshot() {
        guard let device = selected else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(device.name) screenshot.png"
        panel.allowedContentTypes = [.png]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        withSession { [weak self] session in
            try await session.screenshot(path: url.path)
            self?.announce("Saved the screenshot as \(url.lastPathComponent).", tone: .success)
        }
    }
}
