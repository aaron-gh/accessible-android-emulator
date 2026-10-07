package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.ArrayAdapter
import android.widget.CheckBox
import android.widget.EditText
import android.widget.Spinner
import org.json.JSONObject

/** The device's log, filtered by level and text, with new lines as they're written. */
class LogActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private lateinit var search: EditText
    private lateinit var level: Spinner
    private lateinit var pause: CheckBox
    private lateinit var announce: CheckBox
    private val levels = listOf(null to "All levels", "info" to "Info and above", "warning" to "Warnings and errors", "error" to "Errors")
    private var seen = 0L
    private var filter = ""
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
        ui.heading("Device Log")
        search = ui.field("Search")
        val levelLabel = ui.text("Level")
        level = Spinner(this).apply {
            id = android.view.View.generateViewId()
            levelLabel.labelFor = id
            adapter = ArrayAdapter(this@LogActivity, android.R.layout.simple_spinner_item, levels.map { it.second }).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            ui.column.addView(this)
        }
        pause = CheckBox(this).apply { text = "Pause"; ui.column.addView(this) }
        announce = CheckBox(this).apply { text = "Announce new errors"; ui.column.addView(this) }
        ui.addStatus()
        rows = ui.list("Log lines") {}.second
        ui.button("Clear") {
            rows.clear()
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

    override fun onDestroy() {
        if (::rows.isInitialized) Remote.connection?.call("tools.log.stop", params()) {}
        super.onDestroy()
    }

    private fun read() {
        if (pause.isChecked) return
        val chosen = "${search.text}|${level.selectedItemPosition}"
        if (chosen != filter) {
            // A new filter shows matching lines from the start.
            filter = chosen
            seen = 0
            rows.clear()
        }
        val news = seen != 0L
        val p = params().put("since", seen).put("limit", 300).put("search", search.text.toString())
        levels.getOrNull(level.selectedItemPosition)?.first?.let { p.put("level", it) }
        call("tools.log", p) { result ->
            val json = result as JSONObject
            val entries = json.optJSONArray("entries")
            var errors = 0
            var first = ""
            if (entries != null) {
                for (i in 0 until entries.length()) {
                    val e = entries.getJSONObject(i)
                    rows.add("${e.optString("spoken")}. ${e.optString("process", "Unknown process")}, at ${e.optString("clock")}")
                    seen = maxOf(seen, e.optLong("seq"))
                    if (news && e.optString("level") in setOf("error", "fatal")) {
                        if (errors == 0) first = e.optString("spoken")
                        errors++
                    }
                }
                // Long logs are cut from the top.
                while (rows.count > 2000) rows.remove(rows.getItem(0))
            }
            seen = maxOf(seen, json.optLong("latest"))
            json.optString("problem").takeIf { it.isNotEmpty() && it != "null" }?.let { ui.say(it) }
            if (announce.isChecked && errors > 0) {
                ui.say(if (errors == 1) first else "$first. And ${errors - 1} more errors.")
            }
        }
    }
}
