import AppKit
import AVFoundation

/// AAE's announcements: posted to VoiceOver when it runs, otherwise spoken
/// with AVSpeechSynthesizer in the Spoken Content voice and rate.
///
/// Failures interrupt; other announcements queue.
@MainActor
final class Announcer {
    static let shared = Announcer()

    private let synthesizer = AVSpeechSynthesizer()

    func say(_ text: String, interrupt: Bool) {
        if NSWorkspace.shared.isVoiceOverEnabled {
            // Delay so VoiceOver's reading of the triggering control doesn't
            // interrupt the announcement.
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
            synthesizer.speak(SpokenContent.utterance(text))
        }
    }
}

/// The Spoken Content voice and rate (System Settings, Accessibility).
/// AVSpeechSynthesizer doesn't apply them by default.
enum SpokenContent {
    /// An utterance in the user's Spoken Content voice and rate.
    static func utterance(_ text: String) -> AVSpeechUtterance {
        let utterance = AVSpeechUtterance(string: text)
        let (voiceId, rate) = selection()
        if let voiceId, let voice = AVSpeechSynthesisVoice(identifier: voiceId) {
            utterance.voice = voice
        }
        if let rate {
            utterance.rate = min(max(rate, AVSpeechUtteranceMinimumSpeechRate), AVSpeechUtteranceMaximumSpeechRate)
        }
        return utterance
    }

    /// The selection for the system language, or the first. Stored as
    /// alternating language and selection entries.
    static func selection() -> (voiceId: String?, rate: Float?) {
        guard let list = UserDefaults(suiteName: "com.apple.Accessibility")?
            .array(forKey: "SpokenContentDefaultVoiceSelectionsByLanguage")
        else { return (nil, nil) }
        var choices: [(String, [String: Any])] = []
        var index = 0
        while index + 1 < list.count {
            if let language = list[index] as? String, let choice = list[index + 1] as? [String: Any] {
                choices.append((language, choice))
            }
            index += 2
        }
        let language = Locale.current.language.languageCode?.identifier ?? "en"
        guard let choice = (choices.first { $0.0 == language } ?? choices.first)?.1 else {
            return (nil, nil)
        }
        let rate: Float? = switch choice["rate"] {
        case let number as NSNumber: number.floatValue
        case let text as String: Float(text)
        default: nil
        }
        return (choice["voiceId"] as? String, rate)
    }
}
