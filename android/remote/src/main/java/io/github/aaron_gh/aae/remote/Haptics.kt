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
 * Plays the attached device's vibrations on the phone, as they felt on the
 * device. The computer says what the device played, as Android describes it:
 * screen readers play haptic primitives, such as light ticks and clicks at a
 * given strength with pauses between, and the phone plays the same
 * primitives. Other effects play as a low-amplitude vibration of the same
 * duration.
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
    /** The current vibration was rebuilt, so it ends by itself. */
    private var rebuilt = false
    private val stop = Runnable { vibrator.cancel() }

    fun on(described: String?) {
        main.removeCallbacks(stop)
        started = SystemClock.uptimeMillis()
        val effect = described?.let { build(it) }
        rebuilt = effect != null
        vibrator.vibrate(effect ?: VibrationEffect.createOneShot(MAX_MS, GENTLE))
    }

    fun off(ms: Long) {
        if (rebuilt) return
        val left = started + ms - SystemClock.uptimeMillis()
        if (left > 0) main.postDelayed(stop, left) else vibrator.cancel()
    }

    fun release() {
        main.removeCallbacks(stop)
        vibrator.cancel()
    }

    /** An effect as the phone can play it, from Android's description of it, or null. */
    private fun build(text: String): VibrationEffect? {
        // Primitives: "Primitive{primitive=TICK, scale=0.59, delay=50, …}", or
        // in older descriptions "Primitive=TICK(scale=0.59, pause=50ms)".
        val primitives = PRIMITIVE.findAll(text).mapNotNull { m ->
            val id = primitiveId(m.groupValues[1]) ?: return@mapNotNull null
            Triple(id, m.groupValues[2].toFloatOrNull() ?: 1f, m.groupValues[3].toIntOrNull() ?: 0)
        }.toList()
        if (primitives.isNotEmpty()) {
            if (Build.VERSION.SDK_INT >= 30 && vibrator.areAllPrimitivesSupported(*primitives.map { it.first }.toIntArray())) {
                val composition = VibrationEffect.startComposition()
                primitives.forEach { (id, scale, delay) -> composition.addPrimitive(id, scale.coerceIn(0f, 1f), delay) }
                return composition.compose()
            }
            // The phone can't play primitives: its own click or tick.
            val strongest = primitives.maxByOrNull { it.second } ?: return null
            return predefined(if (strongest.second > 0.7f) "CLICK" else "TICK")
        }
        // Android's ready-made effects: "Prebaked{effect=CLICK, …}".
        PREBAKED.find(text)?.let { return predefined(it.groupValues[1]) }
        // Plain vibration at set strengths: "Step{amplitude=0.5, duration=40}".
        val steps = STEP.findAll(text).map { m ->
            (m.groupValues[2].toLongOrNull() ?: 0L) to (m.groupValues[1].toFloatOrNull() ?: -1f)
        }.filter { it.first > 0 }.toList()
        if (steps.isNotEmpty() && steps.sumOf { it.first } < MAX_MS) {
            val timings = steps.map { it.first }.toLongArray()
            val amplitudes = steps.map { (_, a) ->
                if (a < 0) VibrationEffect.DEFAULT_AMPLITUDE else (a * 255).toInt().coerceIn(0, 255)
            }.toIntArray()
            return VibrationEffect.createWaveform(timings, amplitudes, -1)
        }
        return null
    }

    private fun predefined(name: String): VibrationEffect? = when (name) {
        "CLICK" -> VibrationEffect.createPredefined(VibrationEffect.EFFECT_CLICK)
        "TICK", "TEXTURE_TICK" -> VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK)
        "DOUBLE_CLICK" -> VibrationEffect.createPredefined(VibrationEffect.EFFECT_DOUBLE_CLICK)
        "HEAVY_CLICK", "THUD" -> VibrationEffect.createPredefined(VibrationEffect.EFFECT_HEAVY_CLICK)
        else -> null
    }

    private fun primitiveId(name: String): Int? = when (name) {
        "CLICK" -> VibrationEffect.Composition.PRIMITIVE_CLICK
        "TICK" -> VibrationEffect.Composition.PRIMITIVE_TICK
        "QUICK_RISE" -> VibrationEffect.Composition.PRIMITIVE_QUICK_RISE
        "SLOW_RISE" -> VibrationEffect.Composition.PRIMITIVE_SLOW_RISE
        "QUICK_FALL" -> VibrationEffect.Composition.PRIMITIVE_QUICK_FALL
        "LOW_TICK" -> if (Build.VERSION.SDK_INT >= 31) VibrationEffect.Composition.PRIMITIVE_LOW_TICK else VibrationEffect.Composition.PRIMITIVE_TICK
        "THUD" -> if (Build.VERSION.SDK_INT >= 31) VibrationEffect.Composition.PRIMITIVE_THUD else VibrationEffect.Composition.PRIMITIVE_CLICK
        "SPIN" -> if (Build.VERSION.SDK_INT >= 31) VibrationEffect.Composition.PRIMITIVE_SPIN else VibrationEffect.Composition.PRIMITIVE_QUICK_RISE
        else -> null
    }

    companion object {
        private const val MAX_MS = 5_000L
        /** For effects that can't be rebuilt: about a third of full strength. */
        private const val GENTLE = 80
        private val PRIMITIVE = Regex("""Primitive(?:\{primitive=|=)([A-Z_]+)[^})]*?scale=([0-9.]+)(?:[^})]*?(?:delay|pause)=(\d+))?""")
        private val PREBAKED = Regex("""Prebaked(?:=|\{effect=)([A-Z_]+)""")
        private val STEP = Regex("""Step\{amplitude=(-?[0-9.]+), duration=(\d+)""")
    }
}
