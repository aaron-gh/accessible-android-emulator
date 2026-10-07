package io.github.aaron_gh.aae.helper

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/**
 * The last 500 utterances the speech relay received, with times. In memory
 * only, and only while the speech log is on.
 */
object SpeechLog {
    private const val MAX = 500
    private val entries = ArrayDeque<Pair<Long, String>>()

    @Synchronized
    fun record(text: String) {
        if (text.isBlank()) return
        entries.addLast(System.currentTimeMillis() to text)
        while (entries.size > MAX) entries.removeFirst()
    }

    /** Utterances after [since] (milliseconds since 1970), as JSON. */
    @Synchronized
    fun since(since: Long): JSONArray {
        val list = JSONArray()
        for ((time, text) in entries) {
            if (time > since) list.put(JSONObject().put("time", time).put("text", text))
        }
        return list
    }

    @Synchronized
    fun clear() = entries.clear()

    // The real speech engine the relay passes requests to.

    private const val PREFS = "aae"
    private const val KEY_TARGET = "speech_relay_target"
    private const val KEY_RECORDING = "speech_log_recording"

    @Volatile private var recording: Boolean? = null

    /** Whether the relay records what's said. On unless AAE said otherwise. */
    fun recording(context: Context): Boolean =
        recording ?: context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(KEY_RECORDING, true)
            .also { recording = it }

    fun setRecording(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY_RECORDING, on).commit()
        recording = on
        if (!on) clear()
    }

    fun target(context: Context): String? =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_TARGET, null)

    fun setTarget(context: Context, engine: String) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY_TARGET, engine).commit()
    }
}
