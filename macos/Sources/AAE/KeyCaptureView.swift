import AppKit
import SwiftUI

/// The view that holds the keyboard in device mode, forwarding every key to
/// Android by its macOS key code.
///
/// Three AppKit behaviours shape it (AVM found each one the hard way):
/// - Command combinations arrive through performKeyEquivalent, not keyDown,
///   and macOS never sends their key-up. They are sent to Android as taps:
///   press and release together, with repeats ignored. Other keys offered to
///   performKeyEquivalent, such as the arrows, are left to keyDown.
/// - A key held while Command is down never gets its key-up either. So any
///   key still held is released whenever a modifier is released.
/// - Modifiers arrive as flagsChanged, with only the new state. Whether a
///   modifier went down or up is read from its device-specific flag.
final class KeyCaptureView: NSView {
    /// Called for each key going down (true) or up (false), by macOS key code.
    var send: ((UInt16, Bool) -> Void)?

    var isCapturing = false {
        didSet {
            if isCapturing {
                window?.makeFirstResponder(self)
            } else {
                releaseAll()
            }
        }
    }

    private var heldKeys = Set<UInt16>()
    private var heldModifiers = Set<UInt16>()

    /// The device-specific modifier flag for each modifier key code.
    private static let modifierMasks: [UInt16: UInt] = [
        0x37: 0x0008, // left Command
        0x36: 0x0010, // right Command
        0x38: 0x0002, // left Shift
        0x3C: 0x0004, // right Shift
        0x3B: 0x0001, // left Control
        0x3E: 0x2000, // right Control
        0x3A: 0x0020, // left Option
        0x3D: 0x0040, // right Option
    ]
    private static let capsLock: UInt16 = 0x39

    override var acceptsFirstResponder: Bool { isCapturing }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Android keyboard")
        DeviceModeLock.shared.register(self)
    }

    override func keyDown(with event: NSEvent) {
        if KeyLog.enabled {
            KeyLog.write("keyDown keyCode=\(event.keyCode) capturing=\(isCapturing)")
        }
        guard isCapturing else { return super.keyDown(with: event) }
        if DeviceModeLock.isEscapeHatch(event) {
            DeviceModeLock.shared.escapePressed()
            return
        }
        heldKeys.insert(event.keyCode)
        send?(event.keyCode, true)
    }

    override func keyUp(with event: NSEvent) {
        guard isCapturing else { return super.keyUp(with: event) }
        heldKeys.remove(event.keyCode)
        send?(event.keyCode, false)
    }

    override func flagsChanged(with event: NSEvent) {
        if KeyLog.enabled {
            KeyLog.write("flagsChanged keyCode=\(event.keyCode) flags=\(String(event.modifierFlags.rawValue, radix: 16)) capturing=\(isCapturing)")
        }
        guard isCapturing else { return super.flagsChanged(with: event) }
        let code = event.keyCode
        if code == Self.capsLock {
            // macOS reports only the new lock state, so send a tap.
            send?(code, true)
            send?(code, false)
            return
        }
        guard let mask = Self.modifierMasks[code] else { return }
        let down = event.modifierFlags.rawValue & mask != 0
        if down {
            heldModifiers.insert(code)
            send?(code, true)
        } else {
            heldModifiers.remove(code)
            releaseHeldKeys()
            send?(code, false)
        }
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if isCapturing, event.type == .keyDown, DeviceModeLock.isEscapeHatch(event) {
            DeviceModeLock.shared.escapePressed()
            return true
        }
        if KeyLog.enabled {
            KeyLog.write("performKeyEquivalent keyCode=\(event.keyCode) capturing=\(isCapturing) firstResponder=\(window?.firstResponder === self)")
        }
        // AppKit also offers keys like the arrows here before keyDown. Only
        // Command combinations belong on this path; anything else returns
        // false so it arrives through keyDown and keyUp, with holds and repeats.
        guard isCapturing, event.type == .keyDown, event.modifierFlags.contains(.command) else { return false }
        if !event.isARepeat {
            send?(event.keyCode, true)
            send?(event.keyCode, false)
        }
        return true
    }

    /// Releases every key and modifier Android thinks is held, so nothing is
    /// left stuck down when the keyboard goes back to the Mac.
    func releaseAll() {
        releaseHeldKeys()
        for code in heldModifiers {
            send?(code, false)
        }
        heldModifiers.removeAll()
    }

    private func releaseHeldKeys() {
        for code in heldKeys {
            send?(code, false)
        }
        heldKeys.removeAll()
    }
}

/// Puts a KeyCaptureView in a SwiftUI layout.
struct KeyCapture: NSViewRepresentable {
    var capturing: Bool
    var send: (UInt16, Bool) -> Void

    func makeNSView(context: Context) -> KeyCaptureView {
        let view = KeyCaptureView()
        view.send = send
        return view
    }

    func updateNSView(_ view: KeyCaptureView, context: Context) {
        view.send = send
        if view.isCapturing != capturing {
            view.isCapturing = capturing
        }
    }
}

/// Logs key events to standard error, for diagnosing keyboard problems. Key
/// codes reveal what was typed, so this is off unless AAE_KEYLOG=1 is set.
enum KeyLog {
    static let enabled = ProcessInfo.processInfo.environment["AAE_KEYLOG"] == "1"

    static func write(_ line: String) {
        FileHandle.standardError.write(Data("key: \(line)\n".utf8))
    }
}
