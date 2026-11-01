import Carbon
import Foundation

/// The Mac's keyboard layout, for choosing the device's.
enum MacKeyboard {
    /// The current input source's ID, such as "com.apple.keylayout.British".
    static func currentID() -> String? {
        guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
              let id = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
        else { return nil }
        return Unmanaged<CFString>.fromOpaque(id).takeUnretainedValue() as String
    }

    /// What each key types on a keyboard layout, by macOS key code: with no
    /// modifier, or with Shift. Dead keys give their own character, such as
    /// "^" on a French keyboard. Uses the current layout unless given one.
    /// Nil when the layout can't be read.
    static func typing(_ source: TISInputSource? = nil) -> ((UInt16, Bool) -> String?)? {
        guard let source = source ?? TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
              let property = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
        else { return nil }
        let data = Unmanaged<CFData>.fromOpaque(property).takeUnretainedValue() as Data
        let keyboardType = UInt32(LMGetKbdType())
        let shift = UInt32(shiftKey >> 8) & 0xFF
        return { keycode, shifted in
            data.withUnsafeBytes { bytes -> String? in
                guard let layout = bytes.baseAddress?.assumingMemoryBound(to: UCKeyboardLayout.self) else { return nil }
                var deadKeys: UInt32 = 0
                var length = 0
                var characters = [UniChar](repeating: 0, count: 4)
                let status = UCKeyTranslate(
                    layout, keycode, UInt16(kUCKeyActionDisplay), shifted ? shift : 0, keyboardType,
                    OptionBits(kUCKeyTranslateNoDeadKeysMask), &deadKeys, characters.count, &length, &characters
                )
                guard status == noErr, length > 0 else { return nil }
                return String(utf16CodeUnits: characters, count: length)
            }
        }
    }

    /// Whether the keyboard is a JIS one, which has keys ANSI and ISO
    /// keyboards don't.
    static var isJIS: Bool {
        KBGetLayoutType(Int16(LMGetKbdType())) == PhysicalKeyboardLayoutType(kKeyboardJIS)
    }

    /// Calls `changed` whenever the user switches input source.
    static func observe(_ changed: @escaping @MainActor () -> Void) -> NSObjectProtocol {
        DistributedNotificationCenter.default().addObserver(
            forName: NSNotification.Name(kTISNotifySelectedKeyboardInputSourceChanged as String),
            object: nil,
            queue: .main
        ) { _ in
            MainActor.assumeIsolated { changed() }
        }
    }
}
