package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONArray
import org.json.JSONObject

/**
 * The device's accessibility services. Choosing one turns it on or off; a
 * screen reader turned on becomes the device's screen reader.
 */
class ServicesActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private var services = listOf<JSONObject>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Accessibility Services")
        ui.text("Choose a service to turn it on or off. Screen readers come first: turning one on makes it the screen reader.")
        ui.addStatus()
        rows = ui.list("Services") { toggle(services[it]) }.second
        ui.button("Refresh") { load() }
        ui.show(scroll = false)
        load()
    }

    private fun load() {
        call("tools.services", params()) { result ->
            val array = result as? JSONArray ?: JSONArray()
            services = (0 until array.length()).map { array.getJSONObject(it) }
            rows.clear()
            rows.addAll(services.map { s ->
                val kind = if (s.optBoolean("screen_reader")) "screen reader, " else ""
                "${s.optString("label")}, $kind${if (s.optBoolean("on")) "on" else "off"}"
            })
            if (services.isEmpty()) ui.say("No accessibility services are installed.")
        }
    }

    private fun toggle(service: JSONObject) {
        val on = !service.optBoolean("on")
        val label = service.optString("label")
        if (on && service.optBoolean("screen_reader")) ui.say("Switching to $label.")
        call("tools.service", params().put("component", service.optString("component")).put("on", on)) {
            ui.say(if (on && service.optBoolean("screen_reader")) "$label is now the screen reader." else "$label ${if (on) "on" else "off"}.")
            load()
        }
    }
}
