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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Accessibility Inspector")
        ui.addStatus()
        rows = ui.list("Screen elements and problems") { index ->
            details.getOrNull(index)?.let { ui.message("Details", it) }
        }.second
        ui.button("Refresh") { inspect() }
        ui.button("Copy as Text") {
            getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Screen elements", text))
            ui.say("Copied the screen's elements as text.")
        }
        ui.show(scroll = false)
        inspect()
    }

    private fun inspect() {
        ui.say("Reading the screen.")
        call("tools.inspect", params()) { result ->
            val json = result as JSONObject
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
            ui.say("Read ${list?.length() ?: 0} elements, ${if (problems == 0) "no problems" else "$problems problems, listed after the elements"}.")
        }
    }
}
