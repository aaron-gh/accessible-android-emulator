package io.github.aaron_gh.aae.helper

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import kotlin.math.abs

/**
 * Records from the device's microphone for a moment and says how loud it
 * was, so AAE can check sound reaches the microphone. Nothing is kept.
 */
object MicLevel {
    /** The loudest sample over [ms] milliseconds, from 0 to 32767, or -1 if recording failed. */
    fun peak(ms: Int): Int {
        val rate = 16_000
        val minimum = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (minimum <= 0) return -1
        val record = try {
            AudioRecord(MediaRecorder.AudioSource.MIC, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, minimum * 4)
        } catch (_: Exception) {
            return -1
        }
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            return -1
        }
        var peak = 0
        try {
            record.startRecording()
            val buffer = ShortArray(minimum)
            var left = rate * ms / 1000
            while (left > 0) {
                val read = record.read(buffer, 0, minOf(buffer.size, left))
                if (read <= 0) break
                for (i in 0 until read) peak = maxOf(peak, abs(buffer[i].toInt()))
                left -= read
            }
        } finally {
            record.stop()
            record.release()
        }
        return peak
    }
}
