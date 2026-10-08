import AppKit
import CoreLocation
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
    /// A screen reader build waiting for the user to choose devices.
    @Published var screenReaderBuildQuestion: ScreenReaderBuildQuestion?
    /// Apps waiting for the user to choose which devices to install them on.
    @Published var installQuestion: InstallQuestion?
    /// An installed app's parts waiting for the user to choose about.
    @Published var partsQuestion: PartsQuestion?
    /// The selected device's accessibility services, while their window is open.
    @Published private(set) var services: [ServiceInfo] = []
    @Published private(set) var servicesDevice: String?
    @Published private(set) var loadingServices = false
    /// The selected device's apps, while the Apps window is open.
    @Published private(set) var apps: [AppInfo] = []
    @Published private(set) var appsDevice: String?
    @Published private(set) var loadingApps = false
    @Published var showSystemApps = false { didSet { loadApps() } }
    /// The app whose permissions are being shown, and them.
    @Published var permissionsShown: (app: AppInfo, permissions: AppPermissions)?
    @Published var showingOpenLink = false
    /// Build outputs being watched for new builds to install.
    @Published private(set) var watches: [BuildWatch] = []
    @Published var showingSendIntent = false
    /// The last self-test's results.
    @Published private(set) var selfTestResults: [SelfTestRow] = []
    @Published private(set) var selfTestRunning = false
    /// The selected device's snapshots, while the Snapshots window is open.
    @Published private(set) var snapshots: [SnapshotInfo] = []
    @Published private(set) var snapshotsDevice: String?
    @Published private(set) var loadingSnapshots = false
    /// True until the emulator and SDK tools AAE needs are installed.
    @Published private(set) var needsSetup = false
    /// What setting up still needs, once read from Google's list.
    @Published private(set) var setupStatus: SetupStatus?
    @Published private(set) var setupError: String?
    @Published private(set) var settingUp = false
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
    @Published var selection: String? {
        didSet { if selection != oldValue { applyAudioFocus() } }
    }
    /// The window device mode is in: "main", or a device's own window by
    /// the device's id.
    @Published private(set) var deviceModeHost: String?
    /// The device whose own window is in front, if one is.
    var keyDeviceWindow: String?
    static let playOnlyInUseKey = "playOnlyDeviceInUse"
    /// What each busy device is doing, by device id.
    @Published private(set) var busy: [String: String] = [:]
    /// The latest announcement, also shown in the window.
    @Published private(set) var status = ""
    /// The device whose keyboard is captured, if any.
    @Published private(set) var deviceModeID: String?
    /// In device mode, whether keys perform gestures instead of typing.
    @Published private(set) var gestureMode = false
    private let gestureKeys = GestureKeys()
    /// Where gestures happen, in pixels of the screen as the user sees it;
    /// nil is the middle of the screen.
    private var touchPoint: ScreenPoint?
    /// What was last said to be at the touch point.
    private var touchLabel: String?
    /// The item Tab last moved to. Items can share a centre, such as a
    /// widget and the date inside it, so it's found again by its label and
    /// edges, not by the touch point.
    private var touchItem: TouchTarget?
    /// The gesture being performed, so the next one waits for it.
    private var gestureTask: Task<Void, Never>?
    @Published var showingNewDevice = false
    @Published var renaming: DeviceInfo?
    @Published var cloning: DeviceInfo?
    @Published var editingHardware: DeviceInfo?

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
    /// Devices hearing the Mac's microphone.
    @Published private(set) var microphones: Set<String> = []
    /// Devices with a sound file waiting for, or playing into, their microphone.
    @Published private(set) var playingFiles: Set<String> = []
    /// Devices whose missing sound has been mentioned, so it's said once.
    private var soundProblems: Set<String> = []
    private var soundWatch: Task<Void, Never>?
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
        setUpGestureKeys()
        watchSound()
        watchDevices()
        needsSetup = engine?.needsSetup() ?? false
        if needsSetup {
            checkSetup()
        } else {
            mentionUpdates()
        }
        defaultsObserver = NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.correctPitchChanged()
                self?.applyAudioFocus()
            }
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
                VersionInfo(
                    id: $0.sysdir.trimmingCharacters(in: CharacterSet(charactersIn: "/")).replacingOccurrences(of: "/", with: ";"),
                    api: $0.api, preview: false, tag: "", description: $0.description, installed: true, size: "", sysdir: $0.sysdir
                )
            }
        }
        if selected == nil {
            selection = devices.first?.id
        }
    }

    /// `log`: failures go in AAE's log unless they're expected, such as the
    /// end of the screen.
    func announce(_ text: String, tone: Tone = .info, log: Bool = true) {
        if tone == .failure && log {
            engine?.logProblem(message: text)
        }
        status = text
        tone.play()
        Announcer.shared.say(text, interrupt: tone == .failure)
    }

    var defaultScreenReader: String? { engine?.defaultScreenReader() }

    // MARK: - Setting up the SDK

    /// Reads what setting up still needs, from Google's list of tools.
    func checkSetup(refresh: Bool = false) {
        guard let engine else { return }
        setupError = nil
        Task {
            do {
                setupStatus = try await engine.setupStatus(refresh: refresh)
            } catch {
                setupError = error.localizedDescription
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    /// Says, at most once a day, when tools AAE installed have updates. Quiet
    /// when offline or when there are none.
    private func mentionUpdates() {
        guard let engine else { return }
        let key = "lastUpdateMention"
        if let last = UserDefaults.standard.object(forKey: key) as? Date, Date().timeIntervalSince(last) < 24 * 60 * 60 {
            return
        }
        Task {
            guard let status = try? await engine.setupStatus(refresh: false) else { return }
            setupStatus = status
            guard !status.updates.isEmpty else { return }
            UserDefaults.standard.set(Date(), forKey: key)
            let names = status.updates.map(\.name).joined(separator: " and ")
            // After the window has appeared, so VoiceOver reads it.
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            announce("An update is available for \(names). Install it from Android Versions, in the File menu.")
        }
    }

    /// Downloads the missing tools (and with `update`, newer versions),
    /// asking for Google's licence first if needed.
    func runSetup(update: Bool = false) {
        guard let engine, !settingUp else { return }
        settingUp = true
        setupError = nil
        Task {
            defer { settingUp = false }
            do {
                if let licence = try await engine.toolsLicence(update: update) {
                    guard await askLicence(licence, for: "the Android emulator and tools") else {
                        announce("Licence declined. Nothing was downloaded.")
                        return
                    }
                    try engine.acceptLicence(licence: licence)
                }
                let what = update ? "the updates" : "the Android emulator and tools"
                download = (what, 0)
                announce("Downloading \(what). Command Shift I reports progress.")
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
                try await engine.installTools(update: update, listener: relay)
                download = nil
                needsSetup = engine.needsSetup()
                setupStatus = try? await engine.setupStatus(refresh: false)
                refresh()
                loadVersions()
                if update {
                    announce("Updated.", tone: .success)
                } else {
                    announce("AAE is set up. Next, create a device with Command N.", tone: .success)
                }
            } catch {
                download = nil
                setupError = error.localizedDescription
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

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
    /// Whether New Device offers previews of upcoming Android releases. Remembered.
    static let includePreviewsKey = "includePreviews"
    @Published var includePreviews = UserDefaults.standard.bool(forKey: AppModel.includePreviewsKey) {
        didSet {
            UserDefaults.standard.set(includePreviews, forKey: Self.includePreviewsKey)
            if includePreviews != oldValue { loadVersions() }
        }
    }

    func loadVersions(refresh: Bool = false) {
        guard let engine else { return }
        Task {
            do {
                versions = try await engine.versions(refresh: refresh, includePreviews: includePreviews)
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
            if let licence = try await engine.licenceToAccept(id: version.id) {
                guard await askLicence(licence, for: version.description) else {
                    announce("Licence declined. \(version.description) was not downloaded.")
                    return nil
                }
                try engine.acceptLicence(licence: licence)
            }
            download = (version.description, 0)
            announce("Downloading \(version.description), \(version.size). Command Shift I reports progress.")
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
            let image = try await engine.installVersion(id: version.id, listener: relay)
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
                checkSoundAfterStart(id)
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

    /// Restarts Android on the selected device, keeping everything on it.
    func restart() {
        guard let engine, let device = selected, busy[device.id] == nil else { return }
        guard device.running else {
            announce("\(device.name) is not running. Start it first.", tone: .failure)
            return
        }
        if deviceModeID == device.id {
            leaveDeviceMode()
        }
        let id = device.id
        busy[id] = "Restarting"
        // Android's audio goes away while it restarts; listen again after.
        endSession(id)
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                try await engine.restartDevice(id: id, listener: relay)
                _ = try await session(for: id)
                checkSoundAfterStart(id)
                Tone.success.play()
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
        }
    }

    /// Starts the selected device without its quick-boot snapshot, stopping
    /// it first if it's running.
    func coldBoot() {
        guard let engine, let device = selected, busy[device.id] == nil else { return }
        if deviceModeID == device.id {
            leaveDeviceMode()
        }
        let id = device.id
        busy[id] = "Cold booting"
        endSession(id)
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                try await engine.coldBootDevice(id: id, listener: relay)
                _ = try await session(for: id)
                checkSoundAfterStart(id)
                Tone.success.play()
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
        }
    }

    /// Asks, then wipes the selected device back to its first-boot state and
    /// sets it up again with its screen reader.
    func wipe() {
        guard let engine, let device = selected, busy[device.id] == nil else { return }
        let alert = NSAlert()
        alert.messageText = "Wipe \(device.name)?"
        alert.informativeText = "Its apps, data and snapshots are deleted, and it's set up again as it was when first created, with its screen reader. It keeps its name, hardware and volume. This can't be undone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Wipe")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        if deviceModeID == device.id {
            leaveDeviceMode()
        }
        let id = device.id
        busy[id] = "Wiping"
        endSession(id)
        announce("Wiping \(device.name). Setting it up again takes a few minutes.")
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                try await engine.wipeDevice(id: id, listener: relay)
                _ = try await session(for: id)
                Tone.success.play()
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
        }
    }

    /// The devices being stopped, by name.
    var stoppingNames: [String] {
        busy.filter { $0.value == "Stopping" }.keys.map { id in devices.first { $0.id == id }?.name ?? id }
    }

    /// Set while quitting waits for devices to stop.
    var quitWhenStopped: (() -> Void)?

    func stop(_ id: String? = nil) {
        guard let engine, let id = id ?? selection, busy[id] == nil else { return }
        if deviceModeID == id {
            leaveDeviceMode()
        }
        let name = devices.first { $0.id == id }?.name ?? "The device"
        busy[id] = "Stopping"
        announce("Stopping \(name).")
        endSession(id)
        Task {
            do {
                try await engine.stopDevice(id: id)
                announce("\(name) is stopped.", tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
            refresh()
            if let quit = quitWhenStopped, stoppingNames.isEmpty {
                quitWhenStopped = nil
                quit()
            }
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

    /// Exports the selected device, stopped, to one file for another Mac or PC.
    func exportDevice() {
        guard let engine, let device = selected, busy[device.id] == nil else { return }
        guard !device.running else {
            announce("Stop \(device.name) to export it.", tone: .failure)
            return
        }
        let panel = NSSavePanel()
        panel.message = "Export \(device.name), with its apps, data and named snapshots, to import on another computer with the same kind of processor."
        panel.nameFieldStringValue = "\(device.name).aaedevice"
        panel.allowedContentTypes = [UTType(filenameExtension: "aaedevice") ?? .zip]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let id = device.id
        busy[id] = "Exporting"
        announce("Exporting \(device.name).")
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                let said = try await engine.exportDevice(id: id, path: url.path, listener: relay)
                announce(said, tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
            busy[id] = nil
        }
    }

    /// Imports a device exported from AAE.
    func importDevice() {
        guard let engine else { return }
        let panel = NSOpenPanel()
        panel.message = "Choose a device exported from AAE."
        panel.allowedContentTypes = [UTType(filenameExtension: "aaedevice") ?? .zip]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        announce("Importing \(url.deletingPathExtension().lastPathComponent).")
        let relay = ProgressRelay { [weak self] message in self?.announce(message) }
        Task {
            do {
                let device = try await engine.importDevice(path: url.path, listener: relay)
                refresh()
                selection = device.id
                announce("Imported \(device.name).", tone: .success)
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
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

    /// Following the screen: the inspector reads it again whenever it
    /// changes and has been still for half a second.
    @Published private(set) var followingScreen = false
    private var followTask: Task<Void, Never>?

    func followScreen(_ on: Bool) {
        followTask?.cancel()
        followTask = nil
        followingScreen = on
        guard on, let device = selected, device.running else {
            if on { announce("Start the device first.", tone: .failure) }
            followingScreen = false
            return
        }
        announce("Following the screen.")
        followTask = Task { [weak self] in
            var seen: UInt64?
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 500_000_000)
                guard let self, let session = try? await self.session(for: device.id) else { continue }
                guard let changes = try? await session.screenChanges() else {
                    self.announce("This device's AAE helper can't follow the screen.", tone: .failure)
                    self.followingScreen = false
                    return
                }
                if seen == nil { seen = changes.count }
                guard changes.count != seen, changes.quietMs >= 500, !self.inspecting else { continue }
                seen = changes.count
                guard let result = try? await session.inspect() else { continue }
                // Announce only if the printed tree changed.
                if result.text != self.inspection?.text {
                    self.inspection = result
                    self.inspectedDevice = device.name
                    let problems = result.issues.isEmpty ? "no problems" : "\(result.issues.count) problems"
                    self.announce("The screen changed: \(result.rows.count) elements, \(problems).")
                }
            }
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
        announce(on ? "Turning on the speech log." : "Turning off the speech log.")
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
                    // Don't announce lines present when the window opened or
                    // the filter changed.
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
        applyAudioFocus()
        if session.speechBridge() {
            startSpeechBridge(id, session)
        }
        return session
    }

    /// Runs an action on the selected device's session, announcing any failure.
    /// Lets go of a device's connection, with its sound and microphone.
    private func endSession(_ id: String) {
        guard let session = sessions.removeValue(forKey: id) else { return }
        session.stopAudio()
        session.stopSpeechBridge()
        speakers[id] = nil
        if recordings.remove(id) != nil {
            Task { _ = try? await session.stopRecording() }
        }
        if routes.remove(id) != nil {
            _ = session.stopRoute()
        }
        if playingFiles.remove(id) != nil {
            _ = session.stopPlayingIntoMicrophone()
        }
        if microphones.remove(id) != nil {
            Task { _ = try? await session.setMicrophone(on: false) }
        }
    }

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

    /// Gives the keyboard to Android: to type (device mode), or with
    /// `gestures`, to perform screen reader gestures (gesture mode).
    func enterDeviceMode(gestures: Bool = false, host: String = "main") {
        guard let device = selected, !inDeviceMode else { return }
        guard device.running else {
            announce("\(device.name) is not running. Start it first.", tone: .failure)
            return
        }
        Task {
            do {
                let session = try await session(for: device.id)
                // In case Android dropped AAE's full keyboard, which Meta needs.
                try? await session.ensureKeyboardLayout()
                gestureMode = gestures
                deviceModeHost = host
                gestureKeys.reset()
                touchPoint = nil
                touchLabel = nil
                touchItem = nil
                if gestures {
                    // Reading the screen, to move the touch point, needs AAE's helper.
                    try await session.useHelper(on: true)
                }
                deviceModeID = device.id
                activeSession = session
                applyAudioFocus()
                DeviceModeLock.shared.lock { [weak self] in self?.leaveDeviceMode() }
                // Only say the keyboard is Android's once keys really reach it.
                guard await DeviceModeLock.shared.waitUntilCapturing() else {
                    leaveDeviceMode(quietly: true)
                    announce("Couldn't give the keyboard to Android. The main window didn't take focus.", tone: .failure)
                    return
                }
                if gestures {
                    announce("Gesture mode on. Arrows swipe, Space double taps, question mark lists the keys. \(ReturnShortcut.current.spoken) returns to the Mac.")
                } else {
                    announce("Android keyboard on. \(ReturnShortcut.current.spoken) returns to the Mac.")
                }
            } catch {
                announce(error.localizedDescription, tone: .failure)
            }
        }
    }

    func leaveDeviceMode(quietly: Bool = false) {
        guard inDeviceMode else { return }
        DeviceModeLock.shared.unlock()
        // Lifts any held touch, while the session is still there to lift it.
        gestureKeys.reset()
        if gestureMode {
            queueGesture { session in try await session.useHelper(on: false) }
        }
        deviceModeID = nil
        deviceModeHost = nil
        activeSession = nil
        gestureMode = false
        applyAudioFocus()
        if !quietly {
            announce("Mac keyboard on.")
        }
    }

    /// The session keys go to in device mode.
    private(set) var activeSession: Session?

    func sendKey(_ keycode: UInt16, _ down: Bool) {
        if gestureMode {
            gestureKeys.handle(keycode, down)
        } else {
            _ = activeSession?.macKey(keycode: keycode, down: down)
        }
    }

    private func setUpGestureKeys() {
        gestureKeys.act = { [weak self] action in self?.gestureAction(action) }
    }

    private func gestureAction(_ action: GestureAction) {
        if KeyLog.enabled {
            KeyLog.write("gesture action \(action)")
        }
        switch action {
        case let .gesture(name):
            let at = touchPoint
            queueGesture { session in try await session.performGesture(name: name, at: at) }
        case let .press(name):
            let at = touchPoint
            queueGesture { session in try await session.pressGesture(name: name, at: at) }
        case .release:
            queueGesture { session in try await session.releaseGesture() }
        case .nextItem, .previousItem:
            queueGesture { [weak self] session in
                try await self?.moveToItem(next: action == .nextItem, session: session)
            }
        case let .step(dx, dy):
            queueGesture { [weak self] session in
                try await self?.stepTouchPoint(dx: dx, dy: dy, session: session)
            }
        case .centre:
            touchPoint = nil
            touchItem = nil
            queueGesture { [weak self] session in
                try await self?.sayTouchPoint(session: session, prefix: "Middle of the screen.")
            }
        case .whereIsIt:
            queueGesture { [weak self] session in
                try await self?.sayTouchPoint(session: session, prefix: nil)
            }
        case .details:
            queueGesture { [weak self] session in
                try await self?.sayDetails(session: session)
            }
        case .help:
            announce(GestureKeys.helpText)
        case .unknown:
            Tone.failure.play()
        }
    }

    /// Moves the touch point to the next or previous thing on the screen, in
    /// reading order, and says what it is.
    private func moveToItem(next: Bool, session: Session) async throws {
        let screen = try await session.touchTargets()
        let targets = screen.targets
        guard !targets.isEmpty else {
            announce("There's nothing on the screen to touch.", tone: .failure)
            return
        }
        var index: Int
        if let point = touchPoint {
            let item = touchItem.flatMap { item in
                targets.firstIndex { $0.label == item.label && $0.left == item.left && $0.top == item.top && $0.right == item.right && $0.bottom == item.bottom }
            }
            if let current = item ?? Self.targetIndex(at: point, in: targets) {
                index = current + (next ? 1 : -1)
            } else if next {
                // Between items: the next one down the screen.
                index = targets.firstIndex { $0.top >= point.y } ?? targets.count
            } else {
                index = targets.lastIndex { $0.bottom <= point.y } ?? -1
            }
        } else {
            index = next ? 0 : targets.count - 1
        }
        guard targets.indices.contains(index) else {
            announce(next ? "End of the screen." : "Start of the screen.", tone: .failure, log: false)
            return
        }
        let target = targets[index]
        touchPoint = ScreenPoint(x: target.x, y: target.y)
        touchLabel = target.label
        touchItem = target
        announce(target.label)
    }

    /// Moves the touch point a step across the screen, saying what it reaches.
    private func stepTouchPoint(dx: Int, dy: Int, session: Session) async throws {
        let screen = try await session.touchTargets()
        let step = max(min(screen.width, screen.height) / 10, 1)
        let start = touchPoint ?? ScreenPoint(x: screen.width / 2, y: screen.height / 2)
        let x = min(max(start.x + Int32(dx) * step, 0), screen.width - 1)
        let y = min(max(start.y + Int32(dy) * step, 0), screen.height - 1)
        let atEdge = x == start.x && y == start.y
        let point = ScreenPoint(x: x, y: y)
        touchPoint = point
        touchItem = nil
        let under = Self.targetIndex(at: point, in: screen.targets).map { screen.targets[$0] }
        if atEdge {
            Tone.failure.play()
        }
        if let under, under.label == touchLabel {
            // Still on the same item: a tick, not the whole label again.
            if !atEdge { Tone.progress.play() }
        } else {
            announce(under?.label ?? "Nothing. \(Self.position(point, screen))")
        }
        touchLabel = under?.label
    }

    /// Says what's at the touch point, and where it is.
    private func sayTouchPoint(session: Session, prefix: String?) async throws {
        let screen = try await session.touchTargets()
        let point = touchPoint ?? ScreenPoint(x: screen.width / 2, y: screen.height / 2)
        let under = Self.targetIndex(at: point, in: screen.targets).map { screen.targets[$0] }
        touchLabel = under?.label
        let parts = [prefix, under?.label ?? "Nothing.", prefix == nil ? Self.position(point, screen, pixels: true) : nil]
        announce(parts.compactMap { $0 }.joined(separator: " "))
    }

    /// Reads every property of the element at the touch point.
    private func sayDetails(session: Session) async throws {
        let screen = try await session.touchTargets()
        let point = touchPoint ?? ScreenPoint(x: screen.width / 2, y: screen.height / 2)
        let details = try await session.detailsAt(x: point.x, y: point.y)
        announce(details ?? "Nothing. \(Self.position(point, screen, pixels: true))")
    }

    /// The smallest target containing a point.
    private static func targetIndex(at point: ScreenPoint, in targets: [TouchTarget]) -> Int? {
        targets.indices
            .filter { i in
                let t = targets[i]
                return (t.left..<t.right).contains(point.x) && (t.top..<t.bottom).contains(point.y)
            }
            .min { a, b in
                let area = { (t: TouchTarget) in (t.right - t.left) * (t.bottom - t.top) }
                return area(targets[a]) < area(targets[b])
            }
    }

    /// Where a point is, as percentages across and down the screen, and with
    /// `pixels`, its pixel position, as `aae gesture --at` takes it.
    private static func position(_ point: ScreenPoint, _ screen: TouchTargets, pixels: Bool = false) -> String {
        let across = point.x * 100 / max(screen.width, 1)
        let down = point.y * 100 / max(screen.height, 1)
        let at = pixels ? ", pixel \(point.x), \(point.y)" : ""
        return "\(across) percent across, \(down) percent down\(at)."
    }

    /// Runs gesture work after any still going, so gestures happen in order.
    private func queueGesture(_ work: @escaping (Session) async throws -> Void) {
        guard let session = activeSession else { return }
        let previous = gestureTask
        gestureTask = Task { [weak self] in
            await previous?.value
            do {
                try await work(session)
            } catch {
                self?.announce(error.localizedDescription, tone: .failure)
            }
        }
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

    /// Folds or unfolds the selected device, if it's foldable.
    func fold(_ folded: Bool) {
        withSession { [weak self] session in
            let said = try await session.setFolded(folded: folded)
            self?.announce(said)
        }
    }

    /// Sets how loud AAE plays the selected device on the Mac, from 0 to 1,
    /// remembered for the device.
    func setVolume(_ volume: Float, announce say: Bool = false) {
        guard let device = selected else { return }
        let volume = min(max(volume, 0), 1)
        withSession { [weak self] session in
            try session.setAudioVolume(volume: volume)
            if say {
                self?.announce("\(device.name) at \(Int((volume * 100).rounded())) percent.")
            }
            self?.refresh()
        }
    }

    /// The Mac's outputs, with the one a device is set to even if it isn't
    /// there now, such as headphones unplugged.
    func audioOutputNames(including chosen: String?) -> [String] {
        var names = audioOutputs()
        if let chosen, !names.contains(chosen) {
            names.append(chosen)
        }
        return names
    }

    /// Plays the selected device through one of the Mac's outputs, or the
    /// default, and remembers it.
    func setAudioOutput(_ output: String?) {
        withSession { [weak self] session in
            guard let self else { return }
            let said = try await session.setAudioOutput(output: output, correctPitch: self.correctPitch)
            self.applyAudioFocus()
            self.announce(said, tone: .success)
            self.refresh()
        }
    }

    /// Turns the selected device's audio up or down by a tenth.
    func stepVolume(up: Bool) {
        guard let device = selected else { return }
        let now = (device.volume * 10).rounded() / 10
        setVolume(now + (up ? 0.1 : -0.1), announce: true)
    }

    /// Speech bridge speakers, by device.
    private var speakers: [String: SpeechBridgeSpeaker] = [:]

    private func startSpeechBridge(_ id: String, _ session: Session) {
        let speaker = SpeechBridgeSpeaker(session: session)
        speakers[id] = speaker
        session.startSpeechBridge(listener: speaker)
    }

    /// Toggles the selected device's speech bridge.
    func toggleSpeechBridge() {
        guard let device = selected else { return }
        let on = !device.speechBridge
        busy[device.id] = on ? "Turning the speech bridge on" : "Turning the speech bridge off"
        let id = device.id
        withSession { [weak self] session in
            guard let self else { return }
            defer { self.busy[id] = nil }
            let said = try await session.setSpeechBridge(on: on)
            if on {
                self.startSpeechBridge(id, session)
            } else {
                self.speakers[id] = nil
            }
            self.refresh()
            self.announce(said, tone: .success)
        }
    }

    /// Sends the Mac's microphone into the selected device, or stops.
    func toggleMicrophone() {
        guard let device = selected else { return }
        let on = !microphones.contains(device.id)
        withSession { [weak self] session in
            guard let self else { return }
            let said = try await session.setMicrophone(on: on)
            if on { self.microphones.insert(device.id) } else { self.microphones.remove(device.id) }
            self.announce(said)
        }
    }

    /// Plays an audio file into the selected device's microphone from the
    /// next recording. Chosen while one is queued or playing, it stops it.
    func playFileIntoMicrophone() {
        guard let device = selected else { return }
        let id = device.id
        if playingFiles.contains(id) {
            if let session = sessions[id], session.stopPlayingIntoMicrophone() {
                playingFiles.remove(id)
                return
            }
            playingFiles.remove(id)
        }
        let panel = NSOpenPanel()
        panel.message = "Choose an audio file to play into \(device.name)'s microphone."
        panel.allowedContentTypes = [.audio]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        withSession { [weak self] session in
            guard let self else { return }
            self.playingFiles.insert(id)
            self.announce("\(url.lastPathComponent) queued for the next recording.")
            do {
                let said = try await session.playIntoMicrophone(path: url.path)
                self.playingFiles.remove(id)
                self.announce(said)
            } catch {
                self.playingFiles.remove(id)
                throw error
            }
        }
    }

    /// Injects a tone into the selected device's microphone and reports
    /// whether it was recorded. No host audio.
    func checkMicrophone() {
        guard let device = selected else { return }
        announce("Checking \(device.name)'s microphone.")
        withSession { [weak self] session in
            let said = try await session.checkMicrophone()
            self?.announce(said, tone: .success)
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
        parts.append(!inDeviceMode ? "The keyboard is on the Mac." : gestureMode ? "Gesture mode is on." : "The keyboard is in Android.")
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
        panel.message = "Choose apps to install."
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data]
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        install(paths: panel.urls.map(\.path))
    }

    /// Installs apps, from Install App or dropped on the window. With more
    /// than one device running, asks which to install on.
    func install(paths: [String]) {
        let apks = paths.filter { $0.lowercased().hasSuffix(".apk") }
        guard !apks.isEmpty else {
            announce("Only app packages, ending in .apk, can be installed.", tone: .failure)
            return
        }
        let running = devices.filter(\.running)
        switch running.count {
        case 0:
            announce("Start a device first, to install apps on it.", tone: .failure)
        case 1:
            install(apks, on: [running[0].id])
        default:
            let chosen = Set(running.map(\.id).filter { $0 == selection })
            installQuestion = InstallQuestion(paths: apks, devices: running, chosen: chosen)
        }
    }

    /// Installs apps on devices, one at a time. An app's parts are asked
    /// about once, on the first device that hasn't an answer, and the same
    /// answers are used on the others.
    func install(_ paths: [String], on ids: [String]) {
        installQuestion = nil
        guard !ids.isEmpty else { return }
        Task {
            var answers: [String: [String: Bool]] = [:]
            for path in paths {
                let file = (path as NSString).lastPathComponent
                for id in ids {
                    let name = devices.first { $0.id == id }?.name ?? "the device"
                    do {
                        let session = try await session(for: id)
                        announce(ids.count > 1 ? "Installing \(file) on \(name)." : "Installing \(file).")
                        let result = try await session.installApk(path: path)
                        let known = answers[result.package] ?? [:]
                        let made = await afterInstall(result, session: session, known: known)
                        answers[result.package, default: [:]].merge(made) { $1 }
                    } catch {
                        announce("\(file) on \(name): \(error.localizedDescription)", tone: .failure)
                    }
                }
            }
        }
    }

    /// Says what was installed, and asks about any of its parts not chosen
    /// about yet on this device, unless `known` already answers them.
    /// Returns the answers given, by component.
    private func afterInstall(_ result: InstallResult, session: Session, known: [String: Bool]) async -> [String: Bool] {
        var said = ["Installed \(result.package)."]
        for part in result.parts {
            if let on = part.choice {
                said.append("\(part.name) is \(on ? "on" : "off"), as you chose before.")
            }
        }
        announce(said.joined(separator: " "), tone: .success)
        let undecided = result.parts.filter { $0.choice == nil }
        guard !undecided.isEmpty else { return [:] }
        var choices = undecided.compactMap { part in known[part.component].map { AppChoice(part: part, on: $0) } }
        let unknown = undecided.filter { known[$0.component] == nil }
        if !unknown.isEmpty {
            choices += await withCheckedContinuation { continuation in
                partsQuestion = PartsQuestion(package: result.package, parts: unknown) { [weak self] choices in
                    self?.partsQuestion = nil
                    continuation.resume(returning: choices)
                }
            }
        }
        do {
            try await session.setAppChoices(choices: choices)
            let on = choices.filter(\.on).map(\.part.name)
            announce(on.isEmpty ? "Left them all off." : "Turned on \(on.joined(separator: ", ")).", tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
        return Dictionary(choices.map { ($0.part.component, $0.on) }, uniquingKeysWith: { $1 })
    }

    // MARK: - Screen reader builds

    /// Chooses a screen reader build, then asks which devices to install it on.
    func installScreenReaderBuild() {
        guard let engine else { return }
        let panel = NSOpenPanel()
        panel.message = "Choose a screen reader build to install."
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            let package = try engine.apkPackage(path: url.path)
            let users = Set(devices.filter { $0.screenReader == package }.map(\.id))
            screenReaderBuildQuestion = ScreenReaderBuildQuestion(path: url.path, package: package, chosen: users)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    /// Installs a screen reader build on devices, one at a time. Stopped
    /// devices get it when they next start.
    func installScreenReaderBuild(_ path: String, on ids: [String]) {
        screenReaderBuildQuestion = nil
        guard let engine, !ids.isEmpty else { return }
        Task {
            for id in ids {
                let name = devices.first { $0.id == id }?.name ?? "the device"
                announce("Installing on \(name).")
                do {
                    let said = try await engine.installScreenReaderBuild(id: id, path: path, replace: false)
                    announce(said, tone: .success)
                } catch {
                    let message = error.localizedDescription
                    guard message.contains("signed differently") else {
                        announce("\(name): \(message)", tone: .failure)
                        continue
                    }
                    // A build signed differently can only replace the old one, losing its settings.
                    let alert = NSAlert()
                    alert.messageText = "Replace the screen reader on \(name)?"
                    alert.informativeText = "This build is signed differently from the one on \(name), so it can't be installed over it. Replacing it removes the old one first, and with it the screen reader's settings."
                    alert.alertStyle = .warning
                    alert.addButton(withTitle: "Skip \(name)")
                    alert.addButton(withTitle: "Replace")
                    guard alert.runModal() == .alertSecondButtonReturn else { continue }
                    do {
                        let said = try await engine.installScreenReaderBuild(id: id, path: path, replace: true)
                        announce(said, tone: .success)
                    } catch {
                        announce("\(name): \(error.localizedDescription)", tone: .failure)
                    }
                }
            }
            refresh()
        }
    }

    // MARK: - Accessibility services

    func loadServices() {
        servicesDevice = selected?.name
        guard let device = selected, device.running else {
            services = []
            return
        }
        loadingServices = true
        withSession { [weak self] session in
            defer { self?.loadingServices = false }
            self?.services = try await session.listServices()
        }
    }

    /// Turns a service on, to stay on, or off. A screen reader turned on
    /// becomes the device's screen reader instead of the one it had.
    func setService(_ service: ServiceInfo, on: Bool) {
        if on && service.screenReader {
            announce("Switching to \(service.label).")
        }
        withSession { [weak self] session in
            try await session.setService(component: service.component, on: on)
            if on && service.screenReader {
                self?.announce("\(service.label) is now the screen reader.", tone: .success)
            } else {
                self?.announce("\(service.label) \(on ? "on" : "off").", tone: .success)
            }
            self?.loadServices()
            self?.refresh()
        }
    }

    // MARK: - Apps

    /// Reads the selected device's apps, for the Apps window.
    func loadApps() {
        appsDevice = selected?.name
        guard let device = selected, device.running else {
            apps = []
            return
        }
        loadingApps = true
        let system = showSystemApps
        withSession { [weak self] session in
            defer { self?.loadingApps = false }
            self?.apps = try await session.listApps(system: system)
        }
    }

    /// Opens a link on the selected device, in an app or whichever Android chooses.
    func openLink(_ link: String, package: String?) {
        let link = link.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !link.isEmpty else { return }
        withSession { [weak self] session in
            try await session.openLink(link: link, package: package)
            self?.announce("Opened \(link).")
        }
    }

    /// Sends an intent on the selected device, and says what Android said.
    func sendIntent(_ intent: IntentInfo) {
        withSession { [weak self] session in
            let said = try await session.sendIntent(intent: intent)
            self?.announce(said.isEmpty ? "Sent." : said, tone: .success)
        }
    }

    func openApp(_ app: AppInfo) {
        withSession { [weak self] session in
            try await session.openApp(package: app.package)
            self?.announce("Opened \(app.label).")
        }
    }

    func forceStopApp(_ app: AppInfo) {
        withSession { [weak self] session in
            try await session.forceStopApp(package: app.package)
            self?.announce("Stopped \(app.label).", tone: .success)
        }
    }

    /// Asks, then deletes an app's data.
    func clearAppData(_ app: AppInfo) {
        guard confirm("Clear \(app.label)'s data?", "It goes back to how it was when first installed: signed out, with its settings and files deleted.", action: "Clear Data") else { return }
        withSession { [weak self] session in
            try await session.clearAppData(package: app.package)
            self?.announce("Cleared \(app.label)'s data.", tone: .success)
        }
    }

    /// Asks, then uninstalls an app.
    func uninstallApp(_ app: AppInfo) {
        guard confirm("Uninstall \(app.label)?", "It and its data are removed from the device.", action: "Uninstall") else { return }
        withSession { [weak self] session in
            try await session.uninstallApp(package: app.package)
            self?.announce("Uninstalled \(app.label).", tone: .success)
            self?.loadApps()
        }
    }

    func showPermissions(_ app: AppInfo) {
        withSession { [weak self] session in
            let permissions = try await session.appPermissions(package: app.package)
            self?.permissionsShown = (app, permissions)
        }
    }

    func setPermission(_ app: AppInfo, _ permission: PermissionInfo, granted: Bool) {
        withSession { [weak self] session in
            try await session.setAppPermission(package: app.package, permission: permission.name, grant: granted)
            self?.announce("\(permission.label): \(granted ? "granted" : "revoked").")
            self?.showPermissions(app)
        }
    }

    func grantAllPermissions(_ app: AppInfo) {
        withSession { [weak self] session in
            let count = try await session.grantAllPermissions(package: app.package)
            self?.announce(count == 0 ? "\(app.label) already has every permission." : "Granted \(count) permissions.", tone: .success)
            self?.showPermissions(app)
        }
    }

    func setAccess(_ app: AppInfo, _ access: AccessInfo, allowed: Bool) {
        withSession { [weak self] session in
            try await session.setAppAccess(package: app.package, kind: access.kind, allowed: allowed)
            self?.announce("\(access.name): \(allowed ? "allowed" : "not allowed").")
            self?.showPermissions(app)
        }
    }

    /// Asks a yes-or-no question, with Cancel the default.
    private func confirm(_ question: String, _ detail: String, action: String) -> Bool {
        let alert = NSAlert()
        alert.messageText = question
        alert.informativeText = detail
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: action)
        return alert.runModal() == .alertSecondButtonReturn
    }

    // MARK: - Watching builds

    /// Chooses an APK or a build folder to watch, then the devices.
    func watchForBuilds() {
        let panel = NSOpenPanel()
        panel.message = "Choose an app's APK, or the folder its builds go in, such as build/outputs/apk."
        panel.canChooseDirectories = true
        panel.canChooseFiles = true
        panel.allowedContentTypes = [UTType(filenameExtension: "apk") ?? .data, .folder]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let running = devices.filter(\.running)
        guard !running.isEmpty else {
            announce("Start a device first, to install builds on it.", tone: .failure)
            return
        }
        let chosen = Set(running.map(\.id).filter { $0 == selection || running.count == 1 })
        installQuestion = InstallQuestion(paths: [url.path], devices: running, chosen: chosen, watch: true)
    }

    /// Installs each new build at a path on the devices, until stopped.
    func startWatching(_ path: String, on ids: [String]) {
        installQuestion = nil
        guard !ids.isEmpty else { return }
        var watch = BuildWatch(path: path, deviceIDs: ids)
        let watchID = watch.id
        watch.task = Task { [weak self] in
            var last = BuildWatch.currentBuild(at: path)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let build = BuildWatch.currentBuild(at: path), build != last else { continue }
                // Wait for it to settle, so a half-written file isn't installed.
                try? await Task.sleep(nanoseconds: 1_200_000_000)
                guard BuildWatch.currentBuild(at: path) == build, let self else { continue }
                last = build
                self.announce("New build of \(build.url.lastPathComponent).")
                if let i = self.watches.firstIndex(where: { $0.id == watchID }) {
                    self.watches[i].lastBuild = Date()
                }
                self.install([build.url.path], on: ids)
            }
        }
        watches.append(watch)
        let names = ids.compactMap { id in devices.first { $0.id == id }?.name }.joined(separator: " and ")
        announce("Watching \((path as NSString).lastPathComponent). Each new build goes on \(names).", tone: .success)
    }

    func stopWatching(_ watch: BuildWatch) {
        watch.task?.cancel()
        watches.removeAll { $0.id == watch.id }
        announce("Stopped watching \((watch.path as NSString).lastPathComponent).")
    }

    // MARK: - Snapshots

    /// Reads the selected device's snapshots, for the Snapshots window.
    func loadSnapshots() {
        snapshotsDevice = selected?.name
        guard let device = selected, device.running else {
            snapshots = []
            return
        }
        loadingSnapshots = true
        withSession { [weak self] session in
            defer { self?.loadingSnapshots = false }
            self?.snapshots = try await session.snapshots()
        }
    }

    func saveSnapshot(name: String, notes: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        announce("Saving snapshot \(name).")
        withSession { [weak self] session in
            try await session.saveSnapshot(name: name, notes: notes)
            self?.announce("Saved snapshot \(name).", tone: .success)
            self?.loadSnapshots()
        }
    }

    /// Asks, then puts the device back as it was in a snapshot.
    func restoreSnapshot(_ snapshot: SnapshotInfo) {
        let alert = NSAlert()
        alert.messageText = "Restore \(snapshot.name)?"
        alert.informativeText = "The device goes back to how it was when this snapshot was taken. Anything since then is lost, unless you save a snapshot of it first."
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Restore")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        announce("Restoring \(snapshot.name).")
        withSession { [weak self] session in
            try await session.loadSnapshot(id: snapshot.id)
            self?.announce("Restored \(snapshot.name).", tone: .success)
            self?.loadSnapshots()
        }
    }

    func updateSnapshot(_ snapshot: SnapshotInfo, name: String, notes: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        withSession { [weak self] session in
            try await session.updateSnapshot(id: snapshot.id, name: name, notes: notes)
            self?.announce("Saved the changes to \(name).", tone: .success)
            self?.loadSnapshots()
        }
    }

    /// Asks, then deletes a snapshot.
    func deleteSnapshot(_ snapshot: SnapshotInfo) {
        let alert = NSAlert()
        alert.messageText = "Delete \(snapshot.name)?"
        alert.informativeText = "This frees \(snapshot.size). It can't be undone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Delete")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        withSession { [weak self] session in
            try await session.deleteSnapshot(id: snapshot.id)
            self?.announce("Deleted \(snapshot.name).", tone: .success)
            self?.loadSnapshots()
        }
    }

    // MARK: - Clipboard

    /// Copies the device's clipboard to the Mac's.
    func copyDeviceClipboard() {
        withSession { [weak self] session in
            let text = try await session.deviceClipboard()
            guard !text.isEmpty else {
                self?.announce("The device's clipboard is empty.")
                return
            }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            self?.announce("Copied from the device: \(Self.preview(text))", tone: .success)
        }
    }

    /// Puts the Mac's clipboard on the device's.
    func sendClipboardToDevice() {
        guard let text = macClipboardText() else { return }
        withSession { [weak self] session in
            try await session.setDeviceClipboard(text: text)
            self?.announce("Sent to the device's clipboard: \(Self.preview(text))", tone: .success)
        }
    }

    /// Types the Mac's clipboard on the device, for fields that block pasting.
    func typeClipboard() {
        guard let text = macClipboardText() else { return }
        withSession { [weak self] session in
            try await session.typeText(text: text)
            self?.announce("Typed \(text.count) characters.", tone: .success)
        }
    }

    private func macClipboardText() -> String? {
        guard let text = NSPasteboard.general.string(forType: .string), !text.isEmpty else {
            announce("The Mac's clipboard has no text.", tone: .failure)
            return nil
        }
        return text
    }

    /// The start of some text, to read out.
    private static func preview(_ text: String) -> String {
        text.count > 80 ? String(text.prefix(80)) + "…" : text
    }

    // MARK: - Battery, location and phone

    /// The selected device's network, for the conditions window.
    @Published var network: NetworkInfo?

    func loadNetwork() {
        guard selected?.running == true else {
            network = nil
            return
        }
        withSession { [weak self] session in
            self?.network = try await session.network()
        }
    }

    /// Changes the network, says how it is now, and shows it.
    private func changeNetwork(_ change: @escaping (Session) async throws -> Void) {
        withSession { [weak self] session in
            try await change(session)
            // Android takes a moment to report changes.
            try? await Task.sleep(nanoseconds: 800_000_000)
            let now = try await session.network()
            self?.network = now
            self?.announce(now.description, tone: .success)
        }
    }

    func setAirplaneMode(_ on: Bool) { changeNetwork { try await $0.setAirplaneMode(on: on) } }
    func setWifi(_ on: Bool) { changeNetwork { try await $0.setWifi(on: on) } }
    func setMobileData(_ on: Bool) { changeNetwork { try await $0.setMobileData(on: on) } }
    func setNetworkSpeed(_ name: String) { changeNetwork { try await $0.setNetworkSpeed(name: name) } }

    func setBattery(level: Int, charging: Bool, health: String) {
        withSession { [weak self] session in
            try await session.setBattery(level: UInt32(max(0, min(level, 100))), charging: charging)
            let said = try await session.setBatteryHealth(health: health)
            self?.announce("Battery at \(level) percent, \(charging ? "charging" : "not charging"). \(said)", tone: .success)
        }
    }

    /// The selected device's language, display and accessibility settings.
    @Published private(set) var deviceSettings: [DeviceSettingInfo] = []

    func loadDeviceSettings() {
        guard selected?.running == true else {
            deviceSettings = []
            return
        }
        withSession { [weak self] session in
            self?.deviceSettings = try await session.deviceSettings()
        }
    }

    /// What new devices start with, in words.
    @Published private(set) var newDeviceSettings = newDeviceSettingsDescription()

    func useSettingsForNewDevices() {
        withSession { [weak self] session in
            let said = try await session.useSettingsForNewDevices()
            self?.newDeviceSettings = said
            self?.announce(said, tone: .success)
        }
    }

    func newDevicesKeepAndroidSettings() {
        do {
            newDeviceSettings = try forgetNewDeviceSettings()
            announce(newDeviceSettings, tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    func changeDeviceSetting(_ name: String, to value: String) {
        withSession { [weak self] session in
            let said = try await session.changeDeviceSetting(name: name, value: value)
            self?.announce(said, tone: .success)
            self?.deviceSettings = try await session.deviceSettings()
        }
    }

    func touchFingerprint(finger: Int) {
        withSession { [weak self] session in
            let said = try await session.touchFingerprint(finger: UInt32(finger))
            self?.announce(said, tone: .success)
        }
    }

    func shake() {
        withSession { [weak self] session in
            try await session.shake()
            self?.announce("Shook the device.", tone: .success)
        }
    }

    /// Sets the device's location from "latitude, longitude", or a place or
    /// address, which the Mac looks up.
    func setLocation(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let numbers = text.split(whereSeparator: { $0 == "," || $0 == " " }).compactMap { Double($0) }
        if numbers.count == 2, abs(numbers[0]) <= 90, abs(numbers[1]) <= 180 {
            applyLocation(latitude: numbers[0], longitude: numbers[1], place: nil)
            return
        }
        announce("Looking up \(text).")
        CLGeocoder().geocodeAddressString(text) { [weak self] places, error in
            Task { @MainActor in
                guard let place = places?.first, let coordinate = place.location?.coordinate else {
                    self?.announce("Couldn't find \(text). Try an address, or latitude and longitude.", tone: .failure)
                    return
                }
                let name = [place.name, place.locality, place.country].compactMap { $0 }.joined(separator: ", ")
                self?.applyLocation(latitude: coordinate.latitude, longitude: coordinate.longitude, place: name)
            }
        }
    }

    private func applyLocation(latitude: Double, longitude: Double, place: String?) {
        withSession { [weak self] session in
            try await session.setLocation(latitude: latitude, longitude: longitude)
            let coordinates = String(format: "%.5f, %.5f", latitude, longitude)
            self?.announce("Location set to \(place.map { "\($0), " } ?? "")\(coordinates).", tone: .success)
        }
    }

    func sendTextMessage(from: String, text: String) {
        guard !text.isEmpty else { return }
        let from = from.isEmpty ? "5551234" : from
        withSession { [weak self] session in
            try await session.sendSms(from: from, text: text)
            self?.announce("Sent a text message from \(from).", tone: .success)
        }
    }

    func phoneCall(_ action: CallAction, number: String) {
        let number = number.isEmpty ? "5551234" : number
        withSession { [weak self] session in
            try await session.phoneCall(action: action, number: number)
            let said: String
            switch action {
            case .ring: said = "\(number) is calling the device."
            case .hangUp: said = "Hung up."
            case .answer: said = "Answered the device's call."
            case .busy: said = "Busy for the device's call."
            case .hold: said = "Call on hold."
            case .resume: said = "Call taken off hold."
            }
            self?.announce(said)
        }
    }

    // MARK: - Sound

    /// With several devices playing, plays only the one in use: the one in
    /// device mode, or else the selected one, whose window is in front. The
    /// others are silenced, without touching the user's own mute. A setting
    /// turns this off.
    func applyAudioFocus() {
        let onlyInUse = UserDefaults.standard.object(forKey: Self.playOnlyInUseKey) as? Bool ?? true
        let inUse = deviceModeID ?? selection
        for (id, session) in sessions {
            session.setBackground(background: onlyInUse && sessions.count > 1 && id != inUse)
        }
    }

    /// Checks the selected device's audio reaches AAE with a muted test tone,
    /// and offers to restart AAE's audio if not.
    func checkAudio() {
        guard let device = selected else { return }
        announce("Checking \(device.name)'s sound.")
        withSession { [weak self] session in
            guard let self else { return }
            let result = try await session.checkAudio(probe: true)
            if result.working {
                self.soundProblems.remove(device.id)
                self.announce(result.message, tone: .success)
                return
            }
            self.announce(result.message, tone: .failure)
            let alert = NSAlert()
            alert.messageText = "Restart \(device.name)'s audio?"
            alert.informativeText = "\(result.message) Restarting AAE's audio reconnects to the device's sound and to the Mac's output. The device itself keeps running."
            alert.addButton(withTitle: "Restart Audio")
            alert.addButton(withTitle: "Not Now")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            try await session.restartAudio(correctPitch: self.correctPitch)
            self.applyAudioFocus()
            let again = try await session.checkAudio(probe: true)
            if again.working {
                self.soundProblems.remove(device.id)
                self.announce("Restarted the audio. \(again.message)", tone: .success)
            } else {
                self.announce("Restarted the audio, but: \(again.message) Restarting the device may help.", tone: .failure)
            }
        }
    }

    /// After a device starts, checks its sound once, without a test tone,
    /// which would mean muting the screen reader's first words.
    private func checkSoundAfterStart(_ id: String) {
        guard let session = sessions[id] else { return }
        Task {
            // Long enough for the screen reader to have said something.
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            guard let result = try? await session.checkAudio(probe: false), !result.working else { return }
            reportSoundProblem(id, result.message)
        }
    }

    private var deviceWatch: Task<Void, Never>?

    /// Every two seconds, notices devices started or stopped elsewhere, such
    /// as with the aae command. A device this app was connected to that was
    /// started again is reconnected, so its sound plays here again; one
    /// that's stopped is let go. Devices it's starting or stopping itself are
    /// left alone.
    private func watchDevices() {
        deviceWatch = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                guard let self, let engine = self.engine,
                      let fresh = try? engine.devices() else { continue }
                if fresh.map({ "\($0.id) \($0.instance ?? "")" }) != self.devices.map({ "\($0.id) \($0.instance ?? "")" }) {
                    self.devices = fresh
                }
                for (id, session) in self.sessions where self.busy[id] == nil {
                    let device = fresh.first { $0.id == id }
                    if device?.instance == session.instance() { continue }
                    let name = device?.name ?? "A device"
                    if self.deviceModeID == id {
                        self.leaveDeviceMode(quietly: true)
                    }
                    self.endSession(id)
                    if let device, device.running {
                        if (try? await self.session(for: id)) != nil {
                            self.announce("\(name) was restarted outside this app. Reconnected.")
                        }
                    } else {
                        self.announce("\(name) was stopped outside this app.")
                    }
                }
            }
        }
    }

    /// Every half minute, checks each device AAE is playing without making a
    /// sound, and says once when one's sound has stopped reaching AAE.
    private func watchSound() {
        soundWatch = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 30_000_000_000)
                guard let self else { return }
                for (id, session) in self.sessions {
                    guard let result = try? await session.checkAudio(probe: false) else { continue }
                    if result.working {
                        self.soundProblems.remove(id)
                    } else {
                        self.reportSoundProblem(id, result.message)
                    }
                }
            }
        }
    }

    private func reportSoundProblem(_ id: String, _ message: String) {
        guard soundProblems.insert(id).inserted else { return }
        let name = devices.first { $0.id == id }?.name ?? "A device"
        announce("\(name): \(message) Choose Check Audio in the Device menu to restart it.", tone: .failure)
    }

    /// Runs the self-test: the core's checks, plus the app's own, keyboard
    /// capture and each open device's sound. Says how it went.
    func runSelfTest() {
        guard let engine, !selfTestRunning else { return }
        selfTestRunning = true
        selfTestResults = []
        announce("Running the self-test.")
        Task {
            var results = await engine.selfTest().map {
                SelfTestRow(name: $0.name, outcome: $0.outcome, detail: $0.detail)
            }
            results.insert(
                DeviceModeLock.canCaptureShortcuts()
                    ? SelfTestRow(name: "Keyboard capture", outcome: .passed, detail: "AAE can switch the Mac's shortcuts off in device mode, so every key reaches Android.")
                    : SelfTestRow(name: "Keyboard capture", outcome: .warning, detail: "macOS didn't answer, so shortcuts like Command-Space may reach the Mac in device mode."),
                at: 0
            )
            for (id, session) in sessions {
                let name = devices.first { $0.id == id }?.name ?? "A device"
                if let check = try? await session.checkAudio(probe: false) {
                    results.append(SelfTestRow(name: "\(name): sound", outcome: check.working ? .passed : .failed, detail: check.message))
                }
            }
            selfTestResults = results
            selfTestRunning = false
            let problems = results.filter { $0.outcome == .failed }
            let warnings = results.filter { $0.outcome == .warning }
            if problems.isEmpty && warnings.isEmpty {
                announce("All \(results.count) checks passed.", tone: .success)
            } else {
                let found = (problems + warnings).map { "\($0.name): \($0.detail)" }.joined(separator: " ")
                announce("\(problems.count) problems and \(warnings.count) warnings. \(found)", tone: problems.isEmpty ? .info : .failure)
            }
        }
    }

    /// Saves a diagnostic report for a bug report, where the user chooses.
    func saveDiagnosticReport() {
        guard let engine else { return }
        let info = Bundle.main.infoDictionary ?? [:]
        let version = "\(info["CFBundleShortVersionString"] as? String ?? "?") (build \(info["CFBundleVersion"] as? String ?? "?"), Mac app)"
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH.mm"
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "AAE report \(formatter.string(from: Date())).txt"
        panel.allowedContentTypes = [.plainText]
        panel.message = "The report has AAE's log and details of this Mac and your devices, with your home folder, computer name and full name taken out. It never includes what you typed on a device. It's plain text, so you can read it before sending it."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let report = engine.diagnosticReport(version: version)
        do {
            try report.write(to: url, atomically: true, encoding: .utf8)
            announce("Saved \(url.lastPathComponent).", tone: .success)
        } catch {
            announce(error.localizedDescription, tone: .failure)
        }
    }

    /// Devices moving along a route.
    @Published private(set) var routes: Set<String> = []

    /// Moves the selected device along a route from a GPX file, or stops.
    func toggleRoute(speed: Double) {
        guard let device = selected else { return }
        let id = device.id
        if routes.contains(id) {
            routes.remove(id)
            if let session = sessions[id], session.stopRoute() { return }
            return
        }
        let panel = NSOpenPanel()
        panel.message = "Choose a GPX route for \(device.name)."
        panel.allowedContentTypes = [UTType(filenameExtension: "gpx") ?? .xml]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        withSession { [weak self] session in
            guard let self else { return }
            let about = try session.describeRoute(path: url.path, speed: speed)
            self.routes.insert(id)
            self.announce("Playing \(url.lastPathComponent): \(about)")
            do {
                let said = try await session.playRoute(path: url.path, speed: speed)
                self.routes.remove(id)
                self.announce(said, tone: .success)
            } catch {
                self.routes.remove(id)
                throw error
            }
        }
    }

    /// Devices whose screen is being recorded.
    @Published private(set) var recordings: Set<String> = []

    /// Starts recording the selected device's screen and sound into a WebM
    /// file, or stops.
    func toggleRecording() {
        guard let device = selected else { return }
        let id = device.id
        if recordings.contains(id) {
            withSession { [weak self] session in
                self?.recordings.remove(id)
                let said = try await session.stopRecording()
                self?.announce(said, tone: .success)
            }
            return
        }
        let panel = NSSavePanel()
        panel.message = "Record the screen and its sound, for up to three minutes."
        panel.nameFieldStringValue = "\(device.name) recording.webm"
        panel.allowedContentTypes = [UTType(filenameExtension: "webm") ?? .movie]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        withSession { [weak self] session in
            let said = try await session.startRecording(path: url.path)
            self?.recordings.insert(id)
            self?.announce(said, tone: .success)
            // The emulator stops by itself at three minutes.
            try? await Task.sleep(nanoseconds: 181_000_000_000)
            if self?.recordings.contains(id) == true, !session.recordingScreen() {
                self?.recordings.remove(id)
                if let said = try? await session.stopRecording() {
                    self?.announce(said)
                }
            }
        }
    }

    /// Opens the selected device's hardware, if it's stopped.
    func editHardware() {
        guard let device = selected else { return }
        guard !device.running else {
            announce("Stop \(device.name) to change its hardware.", tone: .failure)
            return
        }
        editingHardware = device
    }

    func hardware(of device: DeviceInfo) -> HardwareInfo? {
        do {
            return try engine?.deviceHardware(id: device.id)
        } catch {
            announce(error.localizedDescription, tone: .failure)
            return nil
        }
    }

    /// Returns whether it was saved.
    func setHardware(of device: DeviceInfo, to hardware: HardwareInfo) -> Bool {
        do {
            guard let said = try engine?.setDeviceHardware(id: device.id, hardware: hardware) else { return false }
            announce(said, tone: .success)
            return true
        } catch {
            announce(error.localizedDescription, tone: .failure)
            return false
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
