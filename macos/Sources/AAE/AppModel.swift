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

/// Passes download progress from the Rust core to the main thread.
final class DownloadRelay: DownloadListener, @unchecked Sendable {
    private let onPercent: @MainActor (UInt32) -> Void
    private let onStage: @MainActor (String) -> Void

    init(percent: @escaping @MainActor (UInt32) -> Void, stage: @escaping @MainActor (String) -> Void) {
        onPercent = percent
        onStage = stage
    }

    func downloaded(percent: UInt32) {
        Task { @MainActor in onPercent(percent) }
    }

    func stage(message: String) {
        Task { @MainActor in onStage(message) }
    }
}

/// Everything the window shows and every action the user can take.
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var devices: [DeviceInfo] = []
    @Published private(set) var images: [ImageInfo] = []
    /// Every Android version, installed or downloadable.
    @Published private(set) var versions: [VersionInfo] = []
    /// The version being downloaded and how far it has got.
    @Published private(set) var download: (version: String, percent: UInt32)?
    @Published var licenceRequest: LicenceRequest?
    /// The latest accessibility inspection, and the device it's of.
    @Published private(set) var inspection: Inspection?
    @Published private(set) var inspectedDevice: String?
    @Published private(set) var inspecting = false
    /// The speech log of the selected device, while its window is open.
    @Published private(set) var speechLog: [UtteranceInfo] = []
    @Published private(set) var speechLogDevice: String?
    @Published private(set) var speechLogOn = false
    @Published private(set) var speechLogBusy = false
    private var speechLogTask: Task<Void, Never>?
    /// The device log of the selected device, while its window is open.
    @Published private(set) var logEntries: [LogEntryInfo] = []
    @Published private(set) var logProcesses: [String] = []
    @Published private(set) var logDevice: String?
    @Published private(set) var logProblem: String?
    /// Filters for the device log. Empty means everything.
    @Published var logApp = "" { didSet { if logApp != oldValue { reloadLogs() } } }
    @Published var logTag = "" { didSet { if logTag != oldValue { reloadLogs() } } }
    @Published var logLevel: LogLevel? { didSet { if logLevel != oldValue { reloadLogs() } } }
    @Published var logSearch = "" { didSet { if logSearch != oldValue { reloadLogs() } } }
    /// While paused, new lines wait and the list stays still.
    @Published var logPaused = false
    @Published var announceLogErrors = false
    private var logTask: Task<Void, Never>?
    private var logNeedsReload = false
    /// The shell window's transcript: each command and what it printed.
    @Published private(set) var shellTranscript = ""
    @Published private(set) var shellRunning = false
    @Published private(set) var shellHistory: [String] = []
    /// Installed Android versions, with sizes and users, for the Android Versions window.
    @Published private(set) var installedImages: [InstalledImageInfo] = []
    @Published private(set) var loadingImages = false
    /// A device that has no screen reader, which the user is being asked about.
    @Published var screenReaderQuestion: DeviceInfo?
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

    /// Whether to correct older Android versions' pitch. Changed in Settings.
    static let correctPitchKey = "correctPitch"
    private var correctPitch: Bool {
        UserDefaults.standard.object(forKey: Self.correctPitchKey) as? Bool ?? true
    }
    private var appliedCorrectPitch = true
    private var defaultsObserver: NSObjectProtocol?
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
        appliedCorrectPitch = correctPitch
        defaultsObserver = NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.correctPitchChanged() }
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
        if versions.isEmpty {
            versions = images.map {
                VersionInfo(api: $0.api, tag: "", description: $0.description, installed: true, size: "", sysdir: $0.sysdir)
            }
        }
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

    // MARK: - Android versions

    /// Reads the installed Android versions, their sizes and who uses them.
    func loadInstalledImages() {
        guard let engine else { return }
        loadingImages = true
        Task {
            defer { loadingImages = false }
            do {
                installedImages = try await engine.installedImages()
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    /// Asks, then deletes an installed Android version.
    func removeImage(_ image: InstalledImageInfo) {
        guard let engine else { return }
        guard image.devices.isEmpty else {
            let names = image.devices.joined(separator: ", ")
            announce("\(image.description) can't be deleted while devices use it: \(names). Delete those devices first.", tone: .failure)
            return
        }
        let alert = NSAlert()
        alert.messageText = "Delete \(image.description)?"
        var detail = "This deletes its \(image.size) of files. You can download it again later."
        if !image.otherDevices.isEmpty {
            detail += " Other emulator devices, such as Android Studio's, use it and won't start without it: \(image.otherDevices.joined(separator: ", "))."
        }
        alert.informativeText = detail
        alert.alertStyle = .warning
        // Cancel is the default, so a stray Return deletes nothing.
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Delete")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        announce("Deleting \(image.description).")
        Task {
            do {
                let freed = try await engine.removeImage(sysdir: image.sysdir)
                announce("Deleted \(image.description). Freed \(freed).", tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            refresh()
            loadVersions()
            loadInstalledImages()
        }
    }

    /// Loads the list of Android versions, from Google's list at most once a day.
    func loadVersions(refresh: Bool = false) {
        guard let engine else { return }
        Task {
            do {
                versions = try await engine.versions(refresh: refresh)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    /// Shows the licence and waits for the user's answer.
    private func askLicence(_ licence: LicenceInfo, for version: String) async -> Bool {
        await withCheckedContinuation { continuation in
            licenceRequest = LicenceRequest(version: version, licence: licence) { [weak self] accepted in
                self?.licenceRequest = nil
                continuation.resume(returning: accepted)
            }
        }
    }

    /// Downloads a version, asking for its licence first if needed. Returns
    /// where it's installed, or nil if the user declined or it failed.
    private func installVersion(_ version: VersionInfo) async -> String? {
        guard let engine else { return nil }
        do {
            if let licence = try await engine.licenceToAccept(api: version.api, tag: version.tag) {
                guard await askLicence(licence, for: version.description) else {
                    announce("Licence declined. \(version.description) was not downloaded.")
                    return nil
                }
                try engine.acceptLicence(licence: licence)
            }
            download = (version.description, 0)
            announce("Downloading \(version.description), \(version.size). Press Command Shift I to hear how far it's got.")
            let relay = DownloadRelay(
                percent: { [weak self] percent in
                    guard let self, let current = self.download else { return }
                    if percent / 10 > current.percent / 10 {
                        Tone.progress.play()
                    }
                    self.download = (current.version, percent)
                },
                stage: { [weak self] message in self?.status = message }
            )
            let image = try await engine.installVersion(api: version.api, tag: version.tag, listener: relay)
            download = nil
            announce("\(version.description) is installed.", tone: .success)
            loadVersions()
            refresh()
            return image.sysdir
        } catch {
            download = nil
            announce(error.localizedDescription, tone: .failure)
            return nil
        }
    }

    // MARK: - Devices

    func create(name: String, version: VersionInfo, profile: DeviceProfile, screenReader: String?, volumeBoost: Bool) {
        Task {
            var sysdir = version.sysdir
            if !version.installed {
                guard let installed = await installVersion(version) else { return }
                sysdir = installed
            }
            createInstalled(name: name, sysdir: sysdir, profile: profile, screenReader: screenReader, volumeBoost: volumeBoost)
        }
    }

    private func createInstalled(name: String, sysdir: String, profile: DeviceProfile, screenReader: String?, volumeBoost: Bool) {
        guard let engine else { return }
        do {
            let device = try engine.createDevice(name: name, sysdir: sysdir, profile: profile)
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
                refresh()
                if let device = devices.first(where: { $0.id == id }),
                   device.screenReader == nil, !device.screenReaderDeclined {
                    screenReaderQuestion = device
                }
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

    // MARK: - Accessibility inspector

    /// Reads the selected device's screen and checks it.
    func inspect() {
        guard let device = selected else {
            announce("Select a device first.", tone: .failure)
            return
        }
        inspecting = true
        withSession { [weak self] session in
            defer { self?.inspecting = false }
            let result = try await session.inspect()
            self?.inspection = result
            self?.inspectedDevice = device.name
            let count = result.rows.count
            let problems = result.issues.isEmpty ? "no problems" : "\(result.issues.count) problems"
            self?.announce("Read \(count) elements, \(problems).", tone: result.issues.isEmpty ? .success : .info)
        }
        // withSession announces failures; make sure the busy state clears.
        if !(selected?.running ?? false) {
            inspecting = false
        }
    }

    // MARK: - Speech log

    /// Shows the selected device's speech log, checking for new speech every
    /// half second, until stopped.
    func watchSpeechLog() {
        stopWatchingSpeechLog()
        speechLog = []
        speechLogDevice = selected?.name
        speechLogOn = selected?.speechLog ?? false
        guard let device = selected, device.running else { return }
        speechLogTask = Task { [weak self] in
            var since: UInt64 = 0
            while !Task.isCancelled {
                guard let self else { return }
                if let session = try? await self.session(for: device.id) {
                    self.speechLogOn = session.speechLogOn()
                    if let new = try? await session.speechLog(since: since, clear: false), !new.isEmpty {
                        self.speechLog.append(contentsOf: new)
                        since = new.last?.time ?? since
                    }
                }
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
        }
    }

    func stopWatchingSpeechLog() {
        speechLogTask?.cancel()
        speechLogTask = nil
    }

    func setSpeechLog(_ on: Bool) {
        guard let device = selected else { return }
        speechLogBusy = true
        announce(on ? "Turning on the speech log. The screen reader restarts." : "Turning off the speech log.")
        withSession { [weak self] session in
            defer { self?.speechLogBusy = false }
            let message = try await session.setSpeechLog(on: on)
            self?.speechLogOn = session.speechLogOn()
            self?.announce(message, tone: .success)
            self?.refresh()
            if self?.selected?.id == device.id { self?.speechLogDevice = device.name }
        }
    }

    func clearSpeechLog() {
        speechLog = []
        withSession { session in
            _ = try await session.speechLog(since: UInt64.max, clear: true)
        }
    }

    // MARK: - Device log

    /// The most lines the log window shows at once.
    private static let logLimit: UInt32 = 5000

    /// Shows the selected device's log, checking for new lines every half
    /// second, until stopped.
    func watchLogs() {
        stopWatchingLogs()
        logEntries = []
        logProcesses = []
        logProblem = nil
        logDevice = selected?.name
        guard let device = selected, device.running else { return }
        logTask = Task { [weak self] in
            var seen: UInt64 = 0
            var lastAnnouncement = Date.distantPast
            var unannounced: [LogEntryInfo] = []
            guard let session = try? await self?.session(for: device.id) else { return }
            session.startLogs()
            defer { session.stopLogs() }
            while !Task.isCancelled {
                guard let self else { return }
                if self.logNeedsReload {
                    self.logNeedsReload = false
                    seen = 0
                    self.logEntries = []
                    unannounced = []
                }
                if !self.logPaused {
                    let latest = session.logLatest()
                    let new = session.logEntries(since: seen, filter: self.logFilter, limit: Self.logLimit)
                    // Lines already there when the window opened, or the filter
                    // changed, aren't news.
                    let isNews = seen != 0
                    seen = max(latest, new.last?.seq ?? 0)
                    if !new.isEmpty {
                        self.logEntries.append(contentsOf: new)
                        let excess = self.logEntries.count - Int(Self.logLimit)
                        if excess > 0 { self.logEntries.removeFirst(excess) }
                        if isNews, self.announceLogErrors {
                            unannounced += new.filter { $0.level == .error || $0.level == .fatal }
                        }
                    }
                    self.logProcesses = session.logProcesses()
                    self.logProblem = session.logProblem()
                }
                // At most one announcement every two seconds, so a burst of
                // errors doesn't bury everything else.
                if let first = unannounced.first, Date().timeIntervalSince(lastAnnouncement) >= 2 {
                    let more = unannounced.count - 1
                    let suffix = more == 0 ? "" : more == 1 ? ". And 1 more error." : ". And \(more) more errors."
                    self.announce(first.spoken + suffix, tone: .failure)
                    unannounced = []
                    lastAnnouncement = Date()
                }
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
        }
    }

    func stopWatchingLogs() {
        logTask?.cancel()
        logTask = nil
    }

    /// Shows the lines matching the filters again, from the start.
    func reloadLogs() {
        logNeedsReload = true
    }

    /// Forgets the lines read so far, so only new ones show.
    func clearLogs() {
        logEntries = []
        guard let device = selected, let session = sessions[device.id] else { return }
        session.clearLogs()
    }

    private var logFilter: LogFilter {
        func value(_ text: String) -> String? {
            let text = text.trimmingCharacters(in: .whitespaces)
            return text.isEmpty ? nil : text
        }
        return LogFilter(process: value(logApp), tag: value(logTag), level: logLevel, text: value(logSearch))
    }

    // MARK: - Shell

    /// Runs a command in the selected device's shell and adds it, with what it
    /// printed, to the transcript.
    func runShellCommand(_ command: String) {
        let command = command.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !command.isEmpty, !shellRunning else { return }
        shellHistory.removeAll { $0 == command }
        shellHistory.append(command)
        shellRunning = true
        withSession { [weak self] session in
            defer { self?.shellRunning = false }
            let result: CommandResult
            do {
                result = try await session.runCommand(command: command)
            } catch {
                self?.appendShell("$ \(command)\n\(error.localizedDescription)\n")
                throw error
            }
            var output = result.output
            if !output.isEmpty, !output.hasSuffix("\n") { output += "\n" }
            let ending = result.status == 0 ? "" : "Exit status \(result.status).\n"
            self?.appendShell("$ \(command)\n\(output)\(ending)")
            let lines = output.split(separator: "\n", omittingEmptySubsequences: false).count - 1
            let printed = lines == 0 ? "no output" : lines == 1 ? "1 line" : "\(lines) lines"
            if result.status == 0 {
                self?.announce("Done, \(printed).", tone: .success)
            } else {
                self?.announce("Failed with exit status \(result.status), \(printed).", tone: .failure)
            }
        }
        if !(selected?.running ?? false) {
            shellRunning = false
        }
    }

    func clearShell() {
        shellTranscript = ""
    }

    private func appendShell(_ text: String) {
        shellTranscript += text
    }

    // MARK: - Screen readers

    /// Answers the question about a device with no screen reader.
    func setUpScreenReader(_ device: DeviceInfo, source: ScreenReaderSource?) {
        guard let engine else { return }
        screenReaderQuestion = nil
        guard let source else {
            do {
                try engine.declineScreenReader(id: device.id)
                announce("\(device.name) has no screen reader. AAE won't ask again.")
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            return
        }
        if case .backtalk = source {
            announce("Downloading Backtalk.")
        } else {
            announce("Installing the screen reader.")
        }
        busy[device.id] = "Setting up the screen reader"
        Task {
            do {
                let package = try await engine.addScreenReader(id: device.id, source: source)
                announce("\(package) is installed and on.", tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[device.id] = nil
            refresh()
        }
    }

    /// Applies a change to the pitch correction setting to audio already playing.
    private func correctPitchChanged() {
        guard correctPitch != appliedCorrectPitch else { return }
        appliedCorrectPitch = correctPitch
        for session in sessions.values {
            session.stopAudio()
            Task { try? await session.startAudio(correctPitch: correctPitch) }
        }
    }

    // MARK: - Sessions

    private func session(for id: String) async throws -> Session {
        if let session = sessions[id] {
            return session
        }
        guard let engine else { throw AaeError.Failed(message: "AAE's core did not start.") }
        let session = try await engine.openSession(id: id)
        try await session.startAudio(correctPitch: correctPitch)
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
                // Only say the keyboard is Android's once keys really reach it.
                guard await DeviceModeLock.shared.waitUntilCapturing() else {
                    leaveDeviceMode(quietly: true)
                    announce("Couldn't give the keyboard to Android. The main window didn't take focus.", tone: .failure)
                    return
                }
                announce("Android keyboard on. Control Command Escape returns to the Mac.")
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    func leaveDeviceMode(quietly: Bool = false) {
        guard inDeviceMode else { return }
        DeviceModeLock.shared.unlock()
        deviceModeID = nil
        activeSession = nil
        if !quietly {
            announce("Mac keyboard on.")
        }
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
        if let download {
            announce("Downloading \(download.version): \(download.percent)%.")
            return
        }
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
