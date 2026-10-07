package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.ArrayAdapter
import android.widget.CheckBox
import org.json.JSONObject

/**
 * What the device's screen reader said, with times, while recording is on.
 * New speech appears as it happens.
 */
class SpeechLogActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private lateinit var record: CheckBox
    private var since = 0L
    private var busy = false
    private val main = Handler(Looper.getMainLooper())
    private val poll = object : Runnable {
        override fun run() {
            read()
            main.postDelayed(this, 1000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Speech Log")
        record = CheckBox(this).apply {
            text = "Record speech"
            setOnClickListener { setRecording(isChecked) }
            ui.column.addView(this)
        }
        ui.text("While recording, speech goes through AAE's speech log on its way to the device's speech engine. Turning it on or off restarts the screen reader.")
        ui.addStatus()
        rows = ui.list("Speech") {}.second
        ui.button("Clear") {
            rows.clear()
            call("tools.speech_log", params().put("since", Long.MAX_VALUE).put("clear", true))
        }
        ui.show(scroll = false)
    }

    override fun onResume() {
        super.onResume()
        main.post(poll)
    }

    override fun onPause() {
        main.removeCallbacks(poll)
        super.onPause()
    }

    private fun read() {
        if (busy) return
        call("tools.speech_log", params().put("since", since)) { result ->
            val json = result as JSONObject
            if (!busy) record.isChecked = json.optBoolean("on")
            val said = json.optJSONArray("said") ?: return@call
            for (i in 0 until said.length()) {
                val u = said.getJSONObject(i)
                rows.add("${u.optString("text")}, at ${u.optString("clock")}")
                since = maxOf(since, u.optLong("time"))
            }
        }
    }

    private fun setRecording(on: Boolean) {
        busy = true
        record.isEnabled = false
        ui.say(if (on) "Turning on the speech log. The screen reader restarts." else "Turning off the speech log.")
        call("tools.speech_log.set", params().put("on", on)) { message ->
            busy = false
            record.isEnabled = true
            ui.say(message.toString())
        }
    }
}
