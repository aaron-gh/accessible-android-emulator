package io.github.aaron_gh.aae.helper

import android.content.Context
import android.media.AudioManager
import android.os.Build
import android.util.Log
import kotlin.math.roundToInt

/**
 * Sets the accessibility stream volume. Full by default; AAE sets playback
 * volume on the host.
 */
object Volume {
    const val TAG = "AaeHelper"
    private const val PREFS = "aae"
    private const val KEY_PERCENT = "accessibility_volume_percent"

    /** Saves a level from 0 to 100 and applies it. Returns the stream index set. */
    fun set(context: Context, percent: Int): Int {
        val level = percent.coerceIn(0, 100)
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putInt(KEY_PERCENT, level)
            .apply()
        return apply(context, level)
    }

    fun applySaved(context: Context): Int {
        val level = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(KEY_PERCENT, 100)
        return apply(context, level)
    }

    private fun apply(context: Context, percent: Int): Int {
        val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        // Screen readers speak on the accessibility stream from Android 8.0,
        // and on the media stream before that.
        val stream =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) AudioManager.STREAM_ACCESSIBILITY
            else AudioManager.STREAM_MUSIC
        val max = audio.getStreamMaxVolume(stream)
        val index = (max * percent / 100.0).roundToInt().coerceIn(1, max)
        audio.setStreamVolume(stream, index, 0)
        val now = audio.getStreamVolume(stream)
        Log.i(TAG, "Accessibility volume: asked for $percent% ($index of $max), now $now of $max")
        return now
    }
}
