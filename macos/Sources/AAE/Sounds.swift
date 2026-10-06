import AVFoundation
import SwiftUI

/// A short sound before each announcement, so the result is heard before the
/// words. AAE makes its own tones rather than using the Mac's alert sounds,
/// any of which may be the user's "invalid key" sound.
enum Tone {
    case info, success, failure

    /// Whether sounds play. Changed in Settings.
    static let enabledKey = "playSounds"

    @MainActor
    func play() {
        guard UserDefaults.standard.object(forKey: Tone.enabledKey) as? Bool ?? true else { return }
        TonePlayer.shared.play(notes)
    }

    /// Frequency in hertz and length in seconds of each note.
    private var notes: [(Double, Double)] {
        switch self {
        case .info: return [(880, 0.06)]
        case .success: return [(660, 0.06), (990, 0.08)]
        case .failure: return [(330, 0.09), (220, 0.14)]
        }
    }
}

/// Synthesizes the tones: sine waves with a quick fade in and out, so they
/// click less and sound soft.
@MainActor
private final class TonePlayer {
    static let shared = TonePlayer()

    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let format = AVAudioFormat(standardFormatWithSampleRate: 44_100, channels: 1)!

    private init() {
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: format)
        engine.mainMixerNode.outputVolume = 0.35
    }

    func play(_ notes: [(Double, Double)]) {
        if !engine.isRunning {
            do {
                try engine.start()
            } catch {
                return
            }
        }
        let rate = format.sampleRate
        let gap = Int(rate * 0.015)
        let total = notes.reduce(0) { $0 + Int(rate * $1.1) + gap }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(total)) else { return }
        let samples = buffer.floatChannelData![0]
        var index = 0
        for (frequency, length) in notes {
            let count = Int(rate * length)
            let fade = min(count / 4, Int(rate * 0.01))
            for i in 0..<count {
                let envelope = Double(min(i, count - 1 - i, fade)) / Double(max(fade, 1))
                samples[index] = Float(sin(2 * .pi * frequency * Double(i) / rate) * min(envelope, 1))
                index += 1
            }
            for _ in 0..<gap {
                samples[index] = 0
                index += 1
            }
        }
        buffer.frameLength = AVAudioFrameCount(index)
        player.scheduleBuffer(buffer, at: nil, options: .interrupts)
        if !player.isPlaying {
            player.play()
        }
    }
}

/// AAE's settings window.
struct SettingsView: View {
    @AppStorage(Tone.enabledKey) private var playSounds = true

    var body: some View {
        Form {
            Toggle("Play a sound before each announcement", isOn: $playSounds)
        }
        .padding()
        .frame(width: 380)
    }
}
