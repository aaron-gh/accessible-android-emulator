package io.github.aaron_gh.aae.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONObject

/**
 * The accessibility inspector: the device's screen as its screen reader sees
 * it, in reading order with each element's level, then the problems found.
 * Elements with children, and the problems, are sections: choosing one
 * expands or collapses it. Choosing another element, or a long press on any,
 * shows its details.
 */
class InspectorActivity : ToolActivity() {
    private class Row(val key: String, val parent: Int?, val level: Int, val summary: String, val details: String) {
        var children = 0
    }

    private lateinit var rows: ArrayAdapter<String>
    private var all = listOf<Row>()
    /** The rows shown, as indices into [all]. */
    private var shown = listOf<Int>()
    /** Expanded sections, by key, kept when the screen is read again.
     *  Sections start collapsed. */
    private val expanded = HashSet<String>()
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
        val (list, adapter) = ui.list("Screen elements and problems") { index -> choose(index) }
        rows = adapter
        list.setOnItemLongClickListener { _, _, index, _ ->
            shown.getOrNull(index)?.let { ui.message("Details", all[it].details) }
            true
        }
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

    /** The rows whose sections are all expanded, with each section's state. */
    private fun showRows() {
        val visible = mutableListOf<Int>()
        for (i in all.indices) {
            var parent = all[i].parent
            var hidden = false
            while (parent != null) {
                if (all[parent].key !in expanded) hidden = true
                parent = all[parent].parent
            }
            if (!hidden) visible += i
        }
        shown = visible
        rows.clear()
        rows.addAll(visible.map { i ->
            val row = all[i]
            val level = if (row.key.startsWith("problems")) "" else "Level ${row.level + 1}, "
            val state = when {
                row.children == 0 -> ""
                row.key in expanded -> ", ${items(row.children)}, expanded"
                else -> ", ${items(row.children)}, collapsed"
            }
            "$level${row.summary}$state"
        })
    }

    private fun items(count: Int) = if (count == 1) "1 item" else "$count items"
    private fun problems(count: Int) = if (count == 1) "1 problem" else "$count problems"

    /** A section expands or collapses; any other element shows its details. */
    private fun choose(index: Int) {
        val row = all[shown.getOrNull(index) ?: return]
        if (row.children == 0) return ui.message("Details", row.details)
        if (!expanded.remove(row.key)) expanded += row.key
        showRows()
        ui.say(if (row.key in expanded) "Expanded." else "Collapsed.")
    }

    /** Reads the screen. [quietly], while following it, says so only if it changed. */
    private fun inspect(quietly: Boolean = false) {
        if (!quietly) ui.say("Reading the screen.")
        call("tools.inspect", params()) { result ->
            val json = result as JSONObject
            if (quietly && json.optString("text") == text) return@call
            text = json.optString("text")
            val list = json.optJSONArray("rows")
            val rowsRead = mutableListOf<Row>()
            val position = HashMap<Int, Int>()
            if (list != null) {
                for (i in 0 until list.length()) {
                    val row = list.getJSONObject(i)
                    val parent = if (row.isNull("parent")) null else position[row.optInt("parent")]
                    val level = parent?.let { rowsRead[it].level + 1 } ?: 0
                    val summary = row.optString("summary")
                    val lines = row.optJSONArray("details")
                    val details = (listOf(summary) + (0 until (lines?.length() ?: 0)).map { lines!!.getString(it) })
                        .joinToString("\n")
                    // Its place in the tree, so a section stays expanded when the screen changes.
                    val key = (parent?.let { rowsRead[it].key + "/" } ?: "") + summary
                    position[row.optInt("index")] = rowsRead.size
                    parent?.let { rowsRead[it].children++ }
                    rowsRead += Row(key, parent, level, summary, details)
                }
            }
            val issues = json.optJSONArray("issues")
            val problemCount = issues?.length() ?: 0
            if (problemCount > 0) {
                val section = rowsRead.size
                rowsRead += Row("problems", null, 0, "Problems", "${problems(problemCount)} found on the screen.")
                for (i in 0 until problemCount) {
                    val issue = issues!!.getJSONObject(i)
                    val kind = if (issue.optBoolean("error")) "Error" else "Warning"
                    val line = "$kind: ${issue.optString("message")} Element: ${issue.optString("element")}"
                    rowsRead[section].children++
                    rowsRead += Row("problems/$i", section, 1, line, line)
                }
            }
            all = rowsRead
            showRows()
            val problems = issues?.length() ?: 0
            val elements = list?.length() ?: 0
            val what = "${if (elements == 1) "1 element" else "$elements elements"}, " +
                "${if (problems == 0) "no problems" else "${problems(problems)}, in the Problems section"}."
            ui.say(if (quietly) "The screen changed: $what" else "Read $what")
        }
    }
}
