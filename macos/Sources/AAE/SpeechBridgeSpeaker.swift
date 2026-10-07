import AVFoundation
import Foundation

/// Speech bridge output on the Mac: AVSpeechSynthesizer with VoiceOver's
/// voice and rate while VoiceOver runs, otherwise Spoken Content's. Other
/// languages use a system voice for that language. Reports completion and
/// stops to the device.
final class SpeechBridgeSpeaker: NSObject, SpeechBridgeListener, AVSpeechSynthesizerDelegate, @unchecked Sendable {
    private weak var session: Session?
    private let synthesizer = AVSpeechSynthesizer()
    /// The device's id for each utterance being spoken.
    private var ids: [ObjectIdentifier: UInt64] = [:]

    init(session: Session) {
        self.session = session
        super.init()
        synthesizer.delegate = self
    }

    func speak(id: UInt64, text: String, language: String, rate: UInt32, pitch: UInt32) {
        DispatchQueue.main.async { [self] in
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                session?.speechFinished(id: id)
                return
            }
            if synthesizer.isSpeaking {
                synthesizer.stopSpeaking(at: .immediate)
            }
            let utterance = SpokenContent.utterance(text)
            let voiceLanguage = utterance.voice?.language ?? Locale.current.identifier
            if !language.isEmpty, prefix(language) != prefix(voiceLanguage),
               let voice = AVSpeechSynthesisVoice(language: language) ?? AVSpeechSynthesisVoice(language: prefix(language)) {
                utterance.voice = voice
            } else {
                // VoiceOver's voice and rate, when it's running.
                utterance.prefersAssistiveTechnologySettings = true
            }
            ids[ObjectIdentifier(utterance)] = id
            synthesizer.speak(utterance)
        }
    }

    func stop() {
        DispatchQueue.main.async { [self] in
            synthesizer.stopSpeaking(at: .immediate)
        }
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        ended(utterance)
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        ended(utterance)
    }

    private func ended(_ utterance: AVSpeechUtterance) {
        DispatchQueue.main.async { [self] in
            if let id = ids.removeValue(forKey: ObjectIdentifier(utterance)) {
                session?.speechFinished(id: id)
            }
        }
    }

    private func prefix(_ language: String) -> String {
        String(language.split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? "").lowercased()
    }
}
