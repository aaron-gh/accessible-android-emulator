package io.github.aaron_gh.aae.remote

import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager

/**
 * Plays the attached device's vibrations on the phone. The computer says
 * when the device's vibrator turns on, and when it turns off, with how long
 * it was on. Short ones can arrive together, so the phone keeps each on for
 * as long as it lasted on the device.
 */
class Haptics(context: Context) {
    private val vibrator: Vibrator =
        if (Build.VERSION.SDK_INT >= 31) {
            context.getSystemService(VibratorManager::class.java).defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        }
    private val main = Handler(Looper.getMainLooper())
    private var started = 0L
    private val stop = Runnable { vibrator.cancel() }

    fun on() {
        main.removeCallbacks(stop)
        started = SystemClock.uptimeMillis()
        // Long enough for anything a screen reader does; "off" ends it.
        vibrator.vibrate(VibrationEffect.createOneShot(MAX_MS, VibrationEffect.DEFAULT_AMPLITUDE))
    }

    fun off(ms: Long) {
        val left = started + ms - SystemClock.uptimeMillis()
        if (left > 0) main.postDelayed(stop, left) else vibrator.cancel()
    }

    fun release() {
        main.removeCallbacks(stop)
        vibrator.cancel()
    }

    companion object {
        private const val MAX_MS = 5_000L
    }
}
