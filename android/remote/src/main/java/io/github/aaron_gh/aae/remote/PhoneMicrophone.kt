package io.github.aaron_gh.aae.remote

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder

/**
 * Sends the phone's microphone to the attached device in 20 ms frames of
 * 48 kHz mono 16-bit PCM. Uses the voice communication source for echo
 * cancellation of the device's audio.
 */
class PhoneMicrophone(private val send: (ByteArray, Int) -> Unit) {
    @Volatile private var running = true
    private val thread: Thread

    init {
        val piece = RATE / 50 * 2
        @SuppressLint("MissingPermission") // Asked for before it's made.
        val record = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            maxOf(AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT), piece * 4),
        )
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            throw IllegalStateException("The phone's microphone couldn't be opened.")
        }
        thread = Thread({
            val buffer = ByteArray(piece)
            try {
                record.startRecording()
                while (running) {
                    val read = record.read(buffer, 0, piece)
                    if (read > 0) send(buffer, read)
                }
            } finally {
                record.stop()
                record.release()
            }
        }, "AAE microphone").apply { start() }
    }

    fun stop() {
        running = false
    }

    companion object {
        const val RATE = 48_000
    }
}
