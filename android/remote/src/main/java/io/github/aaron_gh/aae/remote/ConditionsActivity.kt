package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.CheckBox
import android.widget.Spinner
import org.json.JSONObject

/** What the device experiences: its battery, fingerprints, motion, where it is, text messages, calls and its network. */
class ConditionsActivity : ToolActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Battery, Location, Phone and Network")
        ui.addStatus()

        ui.heading("Battery")
        val levelLabel = ui.text("Battery level")
        val levels = (100 downTo 0 step 5).toList()
        val level = Spinner(this).apply {
            id = android.view.View.generateViewId()
            levelLabel.labelFor = id
            adapter = ArrayAdapter(this@ConditionsActivity, android.R.layout.simple_spinner_item, levels.map { "$it percent" }).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            ui.column.addView(this)
        }
        val charging = CheckBox(this).apply { text = "Charging"; isChecked = true; ui.column.addView(this) }
        val healthNames = listOf("good", "failed", "dead", "overvoltage", "overheated")
        val health = spinner("Health", listOf("Good", "Failed", "Dead", "Over voltage", "Overheated"))
        ui.button("Set Battery") {
            val percent = levels[level.selectedItemPosition]
            call("tools.battery", params().put("level", percent).put("charging", charging.isChecked)) {
                call("tools.battery.health.set", params().put("health", healthNames[health.selectedItemPosition])) { said ->
                    ui.say("Battery at $percent percent, ${if (charging.isChecked) "charging" else "not charging"}. $said")
                }
            }
        }

        ui.heading("Fingerprint and Motion")
        val finger = spinner("Finger", (1..10).map { "Finger $it" })
        ui.button("Touch Fingerprint Sensor") {
            call("tools.fingerprint", params().put("finger", finger.selectedItemPosition + 1)) { ui.say(it.toString()) }
        }
        ui.text("Enroll fingers in Android's security settings.")
        ui.button("Shake the Device") { call("tools.shake", params()) { ui.say("Shook the device.") } }

        ui.heading("Location")
        val place = ui.field("Place, address, or latitude and longitude")
        ui.button("Set Location") {
            val text = place.text.toString().trim()
            if (text.isEmpty()) return@button
            ui.say("Looking up $text.")
            call("tools.location", params().put("place", text)) { result ->
                val json = result as JSONObject
                val name = json.optString("place").takeIf { it.isNotEmpty() && it != "null" }?.let { "$it, " } ?: ""
                ui.say("Location set to $name%.5f, %.5f.".format(json.optDouble("latitude"), json.optDouble("longitude")))
            }
        }

        ui.heading("Text Message")
        val from = ui.field("From").apply { setText("5551234") }
        val message = ui.field("Message")
        ui.button("Send Text Message") {
            if (message.text.isEmpty()) return@button ui.say("Write a message first.")
            call("tools.sms", params().put("from", from.text.toString()).put("text", message.text.toString())) {
                ui.say("Sent a text message from ${from.text}.")
            }
        }

        ui.heading("Network")
        val airplane = CheckBox(this).apply { text = "Airplane mode"; ui.column.addView(this) }
        val wifi = CheckBox(this).apply { text = "Wi-Fi"; isChecked = true; ui.column.addView(this) }
        val data = CheckBox(this).apply { text = "Mobile data"; isChecked = true; ui.column.addView(this) }
        val speedLabel = ui.text("Speed")
        val speedNames = mutableListOf<String>()
        val speedRows = ArrayAdapter<String>(this, android.R.layout.simple_spinner_item).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        val speed = Spinner(this).apply {
            id = android.view.View.generateViewId()
            speedLabel.labelFor = id
            adapter = speedRows
            ui.column.addView(this)
        }
        val networkStatus = ui.text("")
        fun show(json: JSONObject) {
            airplane.isChecked = json.optBoolean("airplane")
            wifi.isChecked = json.optBoolean("wifi")
            data.isChecked = json.optBoolean("data")
            speedNames.indexOf(json.optString("speed")).takeIf { it >= 0 }?.let { speed.setSelection(it) }
            networkStatus.text = json.optString("description")
        }
        fun change(setting: String, value: Any) {
            call("tools.network.set", params().put(setting, value)) { result ->
                val json = result as JSONObject
                show(json)
                ui.say(json.optString("description"))
            }
        }
        airplane.setOnClickListener { change("airplane", airplane.isChecked) }
        wifi.setOnClickListener { change("wifi", wifi.isChecked) }
        data.setOnClickListener { change("data", data.isChecked) }
        ui.button("Set Speed") { speedNames.getOrNull(speed.selectedItemPosition)?.let { change("speed", it) } }
        call("tools.network.speeds", params()) { result ->
            val array = result as? org.json.JSONArray ?: return@call
            for (i in 0 until array.length()) {
                val s = array.getJSONObject(i)
                speedNames += s.optString("name")
                speedRows.add(s.optString("description").replaceFirstChar { it.uppercase() })
            }
            call("tools.network", params()) { show(it as JSONObject) }
        }

        ui.heading("Phone Call")
        val number = ui.field("Number").apply { setText("5551234") }
        fun phone(label: String, action: String, said: () -> String) = ui.button(label) {
            call("tools.call", params().put("action", action).put("number", number.text.toString())) { ui.say(said()) }
        }
        phone("Call the Device", "ring") { "${number.text} is calling the device." }
        phone("Hang Up", "hang-up") { "Hung up." }
        phone("Hold", "hold") { "Call on hold." }
        phone("Resume", "resume") { "Call taken off hold." }
        ui.text("Outgoing calls:")
        phone("Answer the Device's Call", "answer") { "Answered the device's call." }
        phone("Be Busy", "busy") { "Busy for the device's call." }
        ui.show()
    }

    /** A labelled list to choose from. */
    private fun spinner(label: String, rows: List<String>): Spinner {
        val labelView = ui.text(label)
        return Spinner(this).apply {
            id = android.view.View.generateViewId()
            labelView.labelFor = id
            adapter = ArrayAdapter(this@ConditionsActivity, android.R.layout.simple_spinner_item, rows).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            ui.column.addView(this)
        }
    }
}
