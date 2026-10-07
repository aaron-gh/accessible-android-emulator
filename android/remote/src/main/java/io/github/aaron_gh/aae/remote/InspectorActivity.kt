package io.github.aaron_gh.aae.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONObject

/**
 * The accessibility inspector: the device's screen as its screen reader sees
 * it, in reading order with each element's level, then the problems found.
 * Choosing an element shows its details.
 */
class InspectorActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private var details = listOf<String>()
    private var text = ""
    private lateinit var follow: android.widget.CheckBox
    private var seen: Long? = null
    private var asking = false
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    /** While following the screen, asks the device whether it changed. */
    private val poll = object : Runnable {
        override fun run() {
            if (!follow.isChecked) return
            main.postDelayed(this, 500)
            if (asking) return
            asking = true
            call("tools.screen_changes", params(), failed = { asking = false }) { result ->
                asking = false
                val json = result as? JSONObject ?: run {
                    follow.isChecked = false
                    return@call ui.say("Stopped following the screen: the device's AAE helper can't follow it.")
                }
                val count = json.optLong("count")
                if (seen == null) seen = count
                if (count != seen && json.optLong("quiet_ms") >= 500) {
                    seen = count
                    inspect(quietly = true)
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Accessibility Inspector")
        ui.addStatus()
        rows = ui.list("Screen elements and problems") { index ->
            details.getOrNull(index)?.let { ui.message("Details", it) }
        }.second
        ui.button("Refresh") { inspect() }
        follow = android.widget.CheckBox(this).apply {
            text = "Follow the screen"
            setOnCheckedChangeListener { _, on ->
                seen = null
                main.removeCallbacks(poll)
                if (on) {
                    ui.say("Following the screen.")
                    main.post(poll)
                }
            }
            ui.column.addView(this)
        }
        ui.button("Copy as Text") {
            getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Screen elements", text))
            ui.say("Copied the screen's elements as text.")
        }
        ui.show(scroll = false)
        inspect()
    }

    override fun onPause() {
        main.removeCallbacks(poll)
        super.onPause()
    }

    override fun onResume() {
        super.onResume()
        if (::follow.isInitialized && follow.isChecked) main.post(poll)
    }

    /** Reads the screen. [quietly], while following it, says so only if it changed. */
    private fun inspect(quietly: Boolean = false) {
        if (!quietly) ui.say("Reading the screen.")
        call("tools.inspect", params()) { result ->
            val json = result as JSONObject
            if (quietly && json.optString("text") == text) return@call
            text = json.optString("text")
            val list = json.optJSONArray("rows")
            val depth = HashMap<Int, Int>()
            val shown = mutableListOf<String>()
            val more = mutableListOf<String>()
            if (list != null) {
                for (i in 0 until list.length()) {
                    val row = list.getJSONObject(i)
                    val level = if (row.isNull("parent")) 0 else (depth[row.optInt("parent")] ?: 0) + 1
                    depth[row.optInt("index")] = level
                    shown += "Level ${level + 1}, ${row.optString("summary")}"
                    val lines = row.optJSONArray("details")
                    more += (listOf(row.optString("summary")) +
                        (0 until (lines?.length() ?: 0)).map { lines!!.getString(it) }).joinToString("\n")
                }
            }
            val issues = json.optJSONArray("issues")
            for (i in 0 until (issues?.length() ?: 0)) {
                val issue = issues!!.getJSONObject(i)
                val kind = if (issue.optBoolean("error")) "Error" else "Warning"
                shown += "$kind: ${issue.optString("message")} Element: ${issue.optString("element")}"
                more += shown.last()
            }
            rows.clear()
            rows.addAll(shown)
            details = more
            val problems = issues?.length() ?: 0
            val what = "${list?.length() ?: 0} elements, ${if (problems == 0) "no problems" else "$problems problems, listed after the elements"}."
            ui.say(if (quietly) "The screen changed: $what" else "Read $what")
        }
    }
}
