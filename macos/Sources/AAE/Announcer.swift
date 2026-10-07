import AppKit
import AVFoundation

/// Speaks AAE's announcements on the Mac.
///
/// With VoiceOver running, an announcement is posted to VoiceOver, so it is
/// spoken in the user's own voice and settings. Without VoiceOver (as in
/// device mode, where Android's screen reader is doing the talking) it is
/// spoken with the system voice, so news like "Mac keyboard on" is never lost.
///
/// Failures interrupt whatever is being said; everything else waits its turn.
@MainActor
final class Announcer {
    static let shared = Announcer()

    private let synthesizer = AVSpeechSynthesizer()

    func say(_ text: String, interrupt: Bool) {
        if NSWorkspace.shared.isVoiceOverEnabled {
            // A short delay lets VoiceOver finish reading the menu item or
            // button that caused the announcement, which would otherwise
            // talk over it.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
                NSAccessibility.post(
                    element: NSApp.mainWindow ?? NSApp.keyWindow ?? NSApp as Any,
                    notification: .announcementRequested,
                    userInfo: [
                        .announcement: text,
                        .priority: (interrupt
                            ? NSAccessibilityPriorityLevel.high
                            : NSAccessibilityPriorityLevel.medium).rawValue,
                    ]
                )
            }
        } else {
            if interrupt {
                synthesizer.stopSpeaking(at: .immediate)
            }
            synthesizer.speak(AVSpeechUtterance(string: text))
        }
    }
}
