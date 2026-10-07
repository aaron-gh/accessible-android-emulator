package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Spinner
import org.json.JSONArray
import org.json.JSONObject

/**
 * The device's language, display and accessibility settings, changed without
 * going through its Settings. Each is a list of its choices; Apply Changes
 * sets those changed.
 */
class DeviceSettingsActivity : ToolActivity() {
    private lateinit var lists: LinearLayout
    private lateinit var tags: EditText
    /** Each setting's name, list, the values of its choices, and its value when read. */
    private val shown = mutableListOf<Shown>()

    private class Shown(val name: String, val list: Spinner, val values: List<String>, val was: String)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Display and Language")
        ui.addStatus()
        lists = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; ui.column.addView(this) }
        tags = ui.field("Or language tags, such as fr-CA, or fr-FR,en-US")
        ui.button("Apply Changes") { apply() }
        ui.button("Refresh") { load() }
        ui.show()
        load()
    }

    private fun load() {
        ui.say("Reading the device's settings.")
        call("tools.settings", params()) { result ->
            val array = result as? JSONArray ?: JSONArray()
            lists.removeAllViews()
            shown.clear()
            for (i in 0 until array.length()) {
                val setting = array.getJSONObject(i)
                val choices = setting.optJSONArray("choices") ?: JSONArray()
                val values = (0 until choices.length()).map { choices.getJSONObject(it).optString("value") }
                val labels = (0 until choices.length()).map { choices.getJSONObject(it).optString("label") }
                val label = ui.text(setting.optString("label"))
                ui.column.removeView(label)
                lists.addView(label)
                val list = Spinner(this).apply {
                    id = android.view.View.generateViewId()
                    label.labelFor = id
                    adapter = ArrayAdapter(this@DeviceSettingsActivity, android.R.layout.simple_spinner_item, labels).apply {
                        setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
                    }
                }
                lists.addView(list)
                val was = setting.optString("value")
                values.indexOf(was).takeIf { it >= 0 }?.let { list.setSelection(it) }
                shown.add(Shown(setting.optString("name"), list, values, was))
            }
            ui.say("Choose settings, then Apply Changes.")
        }
    }

    /** Sets each setting changed, one after another, then reads them again. */
    private fun apply() {
        val typed = tags.text.toString().trim()
        val changes = shown.mapNotNull { s ->
            val chosen = if (s.name == "language" && typed.isNotEmpty()) typed else s.values.getOrNull(s.list.selectedItemPosition)
            if (chosen == null || chosen == s.was) null else s.name to chosen
        }
        if (changes.isEmpty()) return ui.say("Nothing has changed.")
        val said = mutableListOf<String>()
        fun next(index: Int) {
            if (index == changes.size) {
                tags.setText("")
                load()
                ui.say(said.joinToString(" "))
                return
            }
            val (name, value) = changes[index]
            call("tools.settings.set", params().put("name", name).put("value", value)) { result ->
                said.add(result.toString())
                next(index + 1)
            }
        }
        next(0)
    }
}
