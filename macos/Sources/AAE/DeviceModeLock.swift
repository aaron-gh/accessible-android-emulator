import AppKit
import Carbon.HIToolbox

/// Device mode: the keyboard belongs to Android until the user presses
/// Control-Command-Escape. Modelled on AVM's focus lock.
///
/// - Control-Command-Escape is a system hotkey, registered only while locked.
///   It fires outside the responder chain, so it always works, and it only
///   ever returns to the Mac. Entering device mode is always deliberate.
/// - App switching, Force Quit and the Dock are blocked while locked, so
///   Command-Tab and friends reach Android instead of the Mac.
/// - If anything takes keyboard focus within the window, such as VoiceOver's
///   cursor landing on a button, focus goes straight back to the capture view.
///   Otherwise Return or Space could press an AAE button by accident.
@MainActor
final class DeviceModeLock {
    static let shared = DeviceModeLock()

    private(set) var isLocked = false
    private weak var captureView: KeyCaptureView?
    private var responderObservation: NSKeyValueObservation?
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private var onEscape: (() -> Void)?

    func register(_ view: KeyCaptureView) {
        captureView = view
        if isLocked {
            enforceFocus()
        }
    }

    /// Hands the keyboard to Android. `onEscape` runs when the user presses
    /// Control-Command-Escape.
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
        registerHotKey()
        enforceFocus()
    }

    /// Gives the keyboard back to the Mac.
    func unlock() {
        guard isLocked else { return }
        isLocked = false
        responderObservation = nil
        unregisterHotKey()
        NSApp.presentationOptions = []
        captureView?.isCapturing = false
        if let window = captureView?.window {
            window.makeFirstResponder(nil)
        }
    }

    private func enforceFocus() {
        guard let view = captureView, let window = view.window else { return }
        view.isCapturing = true
        window.makeFirstResponder(view)
        responderObservation = window.observe(\.firstResponder, options: [.new]) { [weak self] window, _ in
            Task { @MainActor in
                guard let self, self.isLocked, let view = self.captureView else { return }
                if window.firstResponder !== view {
                    window.makeFirstResponder(view)
                }
            }
        }
    }

    fileprivate func escapePressed() {
        guard isLocked else { return }
        onEscape?()
    }

    private func registerHotKey() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(
            GetApplicationEventTarget(),
            { _, _, _ in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { DeviceModeLock.shared.escapePressed() }
                }
                return noErr
            },
            1,
            &spec,
            nil,
            &handlerRef
        )
        let id = EventHotKeyID(signature: OSType(0x4141_4531), id: 1) // "AAE1"
        RegisterEventHotKey(
            UInt32(kVK_Escape),
            UInt32(controlKey | cmdKey),
            id,
            GetApplicationEventTarget(),
            0,
            &hotKeyRef
        )
    }

    private func unregisterHotKey() {
        if let hotKeyRef {
            UnregisterEventHotKey(hotKeyRef)
        }
        if let handlerRef {
            RemoveEventHandler(handlerRef)
        }
        hotKeyRef = nil
        handlerRef = nil
    }
}
