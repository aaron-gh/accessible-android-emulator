package io.github.aaron_gh.aae.helper

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import kotlin.math.PI
import kotlin.math.sin

/**
 * Plays an exact tone, so AAE can measure the pitch that reaches the computer
 * and spot audio played at the wrong speed. Nobody hears it: AAE measures
 * while it isn't playing the device's audio, and the emulator's own audio
 * output is off.
 */
object TestTone {
    /** The output's native sample rate, which Android mixes at. */
    fun nativeRate(): Int = AudioTrack.getNativeOutputSampleRate(AudioManager.STREAM_MUSIC)

    /** Plays [hz] for [seconds] at [rate] samples a second, and returns at once. */
    fun play(hz: Int, rate: Int, seconds: Double) {
        val frames = (rate * seconds).toInt()
        val samples = ShortArray(frames) { i ->
            (sin(2 * PI * hz * i / rate) * Short.MAX_VALUE * 0.8).toInt().toShort()
        }
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    // The accessibility volume, which AAE turns up to full,
                    // so the tone is loud enough to measure.
                    .setUsage(AudioAttributes.USAGE_ASSISTANCE_ACCESSIBILITY)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(rate)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build()
            )
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(frames * 2)
            .build()
        track.write(samples, 0, frames)
        track.setNotificationMarkerPosition(frames)
        track.setPlaybackPositionUpdateListener(object : AudioTrack.OnPlaybackPositionUpdateListener {
            override fun onMarkerReached(t: AudioTrack) = t.release()
            override fun onPeriodicNotification(t: AudioTrack) = Unit
        })
        track.play()
    }
}
