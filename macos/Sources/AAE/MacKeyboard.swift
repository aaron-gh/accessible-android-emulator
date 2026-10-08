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
