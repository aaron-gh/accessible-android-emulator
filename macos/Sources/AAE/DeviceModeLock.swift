import AppKit

// CoreGraphics' private calls for switching system shortcuts on and off, as
// Parallels, VMware and Citrix use. The mode lasts only while this process
// runs, so quitting or crashing gives the Mac its shortcuts back.
@_silgen_name("CGSMainConnectionID")
private func CGSMainConnectionID() -> Int32
@_silgen_name("CGSSetGlobalHotKeyOperatingMode")
private func CGSSetGlobalHotKeyOperatingMode(_ connection: Int32, _ mode: Int32) -> Int32
private let hotKeysEnabled: Int32 = 0
/// Every system shortcut is off except Universal Access ones, such as VoiceOver's.
private let hotKeysExceptUniversalAccess: Int32 = 2

/// Device mode: the keyboard belongs to Android until the user presses
/// Control-Command-Escape. Modelled on AVM's focus lock, with every system
/// shortcut captured too.
///
/// - App switching, Force Quit and the Dock are blocked while locked, so
///   Command-Tab and friends reach Android instead of the Mac.
/// - macOS's other system shortcuts (Spotlight's Command-Space, Mission
///   Control, screenshots, input sources) are switched off while AAE is the
///   active app, so they reach Android too. Accessibility shortcuts, such as
///   Command-F5 for VoiceOver, keep working.
/// - Control-Command-Escape is caught by an event monitor that sees every key
///   before anything else in the app. It is NOT a system hotkey: with system
///   shortcuts switched off, macOS swallows hotkeys, and the way out was lost.
///   It only ever returns to the Mac; entering device mode is always deliberate.
/// - There are two more ways out, so the keyboard can never be trapped: a
///   "Return to the Mac" button, which VoiceOver can press even while keys go
///   to Android, and a watchdog that switches system shortcuts back on if
///   AAE stops responding.
/// - If anything takes keyboard focus within the window, such as VoiceOver's
///   cursor landing on a button, focus goes straight back to the capture view.
///   Otherwise Return or Space could press an AAE button by accident.
/// - The capture view lives in the main window, so that window is brought to
///   the front and kept there. If another AAE window, such as the Speech Log,
///   becomes the key window, keys would go to it instead of Android.
@MainActor
final class DeviceModeLock {
    static let shared = DeviceModeLock()

    private(set) var isLocked = false
    private weak var captureView: KeyCaptureView?
    private var responderObservation: NSKeyValueObservation?
    private var keyWindowToken: NSObjectProtocol?
    private var escapeMonitor: Any?
    private var activationTokens: [NSObjectProtocol] = []
    private var onEscape: (() -> Void)?
    private let watchdog = Watchdog()

    func register(_ view: KeyCaptureView) {
        captureView = view
        if isLocked {
            enforceFocus()
        }
    }

    /// Hands the keyboard to Android. `onEscape` runs when the user asks to
    /// return to the Mac.
    func lock(onEscape: @escaping () -> Void) {
        guard !isLocked else { return }
        self.onEscape = onEscape
        isLocked = true
        NSApp.presentationOptions = [
            .hideDock,
            .hideMenuBar,
            .disableProcessSwitching,
            .disableForceQuit,
            .disableSessionTermination,
        ]
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard Self.isEscapeHatch(event) else { return event }
            MainActor.assumeIsolated { DeviceModeLock.shared.escapePressed() }
            return nil
        }
        enforceFocus()
        captureSystemShortcuts(NSApp.isActive)
        // Give system shortcuts back whenever another app is in front, and take
        // them again when AAE is.
        let center = NotificationCenter.default
        activationTokens = [
            center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { DeviceModeLock.shared.captureSystemShortcuts(true) }
            },
            center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { DeviceModeLock.shared.captureSystemShortcuts(false) }
            },
        ]
        watchdog.start { captureSystemShortcutsFromAnyThread(false) }
    }

    /// Gives the keyboard back to the Mac.
    func unlock() {
        guard isLocked else { return }
        isLocked = false
        watchdog.stop()
        responderObservation = nil
        if let keyWindowToken {
            NotificationCenter.default.removeObserver(keyWindowToken)
        }
        keyWindowToken = nil
        if let escapeMonitor {
            NSEvent.removeMonitor(escapeMonitor)
        }
        escapeMonitor = nil
        activationTokens.forEach(NotificationCenter.default.removeObserver)
        activationTokens = []
        captureSystemShortcuts(false)
        NSApp.presentationOptions = []
        captureView?.isCapturing = false
        if let window = captureView?.window {
            window.makeFirstResponder(nil)
        }
    }

    /// Control-Command-Escape, the way back to the Mac.
    nonisolated static func isEscapeHatch(_ event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        return event.keyCode == 0x35 && flags.contains(.control) && flags.contains(.command)
    }

    /// Returns the keyboard to the Mac, from the escape keys or the button.
    func escapePressed() {
        guard isLocked else { return }
        onEscape?()
    }

    /// True when keys really reach Android: the capture view has keyboard
    /// focus in the key window of the active app.
    var isCapturing: Bool {
        guard isLocked, let view = captureView, let window = view.window else { return false }
        return view.isCapturing && NSApp.isActive && window.isKeyWindow && window.firstResponder === view
    }

    /// Waits up to `timeout` seconds for the keyboard to reach Android.
    func waitUntilCapturing(timeout: TimeInterval = 3) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if isCapturing { return true }
            if isLocked { enforceFocus() }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return isCapturing
    }

    private func enforceFocus() {
        guard let view = captureView, let window = view.window else { return }
        view.isCapturing = true
        NSApp.activate(ignoringOtherApps: true)
        if !window.isKeyWindow {
            window.makeKeyAndOrderFront(nil)
        }
        window.makeFirstResponder(view)
        if keyWindowToken == nil {
            // Another AAE window becoming key, such as the Speech Log, would
            // take the keys; bring the capture window back in front.
            keyWindowToken = NotificationCenter.default.addObserver(
                forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main
            ) { note in
                let other = note.object as? NSWindow
                MainActor.assumeIsolated {
                    let lock = DeviceModeLock.shared
                    guard lock.isLocked, let window = lock.captureView?.window, other !== window else { return }
                    window.makeKeyAndOrderFront(nil)
                    if let view = lock.captureView { window.makeFirstResponder(view) }
                }
            }
        }
        responderObservation = window.observe(\.firstResponder, options: [.new]) { [weak self] window, _ in
            Task { @MainActor in
                guard let self, self.isLocked, let view = self.captureView else { return }
                if window.firstResponder !== view {
                    window.makeFirstResponder(view)
                }
            }
        }
    }

    /// Switches macOS's system shortcuts off (true) or back on (false), keeping
    /// accessibility shortcuts. Only ever switched off while locked.
    private func captureSystemShortcuts(_ capture: Bool) {
        captureSystemShortcutsFromAnyThread(capture && isLocked)
    }
}

private func captureSystemShortcutsFromAnyThread(_ capture: Bool) {
    _ = CGSSetGlobalHotKeyOperatingMode(CGSMainConnectionID(), capture ? hotKeysExceptUniversalAccess : hotKeysEnabled)
}

/// Gives the Mac its system shortcuts back if AAE's main thread stops
/// responding while it holds the keyboard, so a hang can't trap the user.
private final class Watchdog: @unchecked Sendable {
    private let lock = NSLock()
    private var lastBeat = Date()
    private var running = false
    private var timer: Timer?

    @MainActor
    func start(onHang: @escaping @Sendable () -> Void) {
        lock.withLock {
            running = true
            lastBeat = Date()
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.lock.withLock { self.lastBeat = Date() }
        }
        Thread.detachNewThread { [weak self] in
            while let self {
                Thread.sleep(forTimeInterval: 1)
                let (running, stale) = self.lock.withLock { (self.running, Date().timeIntervalSince(self.lastBeat) > 3) }
                if !running { return }
                if stale { onHang() }
            }
        }
    }

    @MainActor
    func stop() {
        lock.withLock { running = false }
        timer?.invalidate()
        timer = nil
    }
}
