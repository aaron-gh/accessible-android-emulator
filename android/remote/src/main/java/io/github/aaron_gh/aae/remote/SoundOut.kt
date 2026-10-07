package io.github.aaron_gh.aae.remote

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack

/**
 * Plays the attached device's sound as it arrives, with as little delay as
 * Android allows. When it can't keep up, it drops sound rather than fall
 * behind, so speech always matches what's happening.
 */
class SoundOut {
    private val track: AudioTrack

    init {
        val format = AudioFormat.Builder()
            .setSampleRate(RATE)
            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .build()
        val minimum = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT)
        track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build()
            )
            .setAudioFormat(format)
            // Room for about 100 ms: more is delay, less breaks up on Wi-Fi.
            .setBufferSizeInBytes(maxOf(minimum, RATE * 4 / 10))
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .build()
        track.play()
    }

    /** 16-bit stereo samples at 48 kHz, little-endian. */
    fun write(bytes: ByteArray, offset: Int, length: Int) {
        track.write(bytes, offset, length - length % 4, AudioTrack.WRITE_NON_BLOCKING)
    }

    fun release() {
        track.pause()
        track.flush()
        track.release()
    }

    companion object {
        const val RATE = 48_000
    }
}
