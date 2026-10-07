package io.github.aaron_gh.aae.remote

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/**
 * One device: starting and stopping it, Android's buttons, keyboard and
 * gesture modes, and managing it. While it runs, its sound and vibrations
 * play on the phone.
 */
class DeviceActivity : ConnectedActivity() {
    private lateinit var id: String
    private var device: JSONObject? = null
    private lateinit var title: TextView
    private lateinit var about: TextView
    private lateinit var startStop: Button
    private val whileRunning = mutableListOf<Button>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        id = intent.getStringExtra("id") ?: return finish()
        title = ui.heading("Device")
        about = ui.text("")
        ui.addStatus()
        startStop = ui.button("Start") { startOrStop() }
        fun running(text: String, action: () -> Unit) = whileRunning.add(ui.button(text, action))
        running("Use Keyboard") { startActivity(Intent(this, KeyboardActivity::class.java)) }
        running("Gesture Mode") { startActivity(Intent(this, GestureActivity::class.java)) }
        running("Testing Tools") { startActivity(Intent(this, ToolsActivity::class.java).putExtra("id", id)) }
        running("Speak Status") { call("device.status", idParams()) { ui.say(it.toString()) } }
        running("Back") { press("back") }
        running("Home") { press("home") }
        running("Recent Apps") { press("recents") }
        running("Notifications") { call("device.notifications", idParams()) }
        running("Quick Settings") { call("device.quick_settings", idParams()) }
        running("Rotate Left") { call("device.rotate", idParams().put("left", true)) { ui.say(it.toString()) } }
        running("Rotate Right") { call("device.rotate", idParams().put("left", false)) { ui.say(it.toString()) } }
        running("Restart") {
            ui.say("Restarting.")
            call("device.restart", idParams()) { ui.say("Restarted."); attach() }
        }
        ui.button("Rename") { rename() }
        ui.button("Copy") { copy() }
        ui.button("Wipe") { wipe() }
        ui.button("Delete") { delete() }
        ui.show()
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    override fun onDestroy() {
        if (Remote.attached == id) Remote.detach()
        super.onDestroy()
    }

    private fun idParams() = JSONObject().put("id", id)

    private fun press(key: String) = call("device.press", idParams().put("key", key))

    private fun refresh(then: () -> Unit = {}) {
        call("devices.list") { result ->
            val array = result as? JSONArray ?: JSONArray()
            device = (0 until array.length()).map { array.getJSONObject(it) }.firstOrNull { it.optString("id") == id }
            val d = device ?: return@call finish()
            title.text = d.optString("name")
            about.text = DevicesActivity.describe(d)
            val running = d.optBoolean("running")
            startStop.text = if (running) "Stop" else "Start"
            whileRunning.forEach { it.isEnabled = running }
            if (running) attach()
            then()
        }
    }

    /** Plays the device's sound and vibrations here while it runs. */
    private fun attach() {
        Remote.attach(id) { result ->
            result.onFailure { ui.say("The device's sound can't play here: ${it.message}") }
        }
    }

    private fun startOrStop() {
        val d = device ?: return
        if (d.optBoolean("running")) {
            Remote.detach()
            ui.say("Stopping ${d.optString("name")}.")
            call("device.stop", idParams()) { refresh { ui.say("Stopped.") } }
        } else {
            ui.say("Starting ${d.optString("name")}.")
            call("device.start", idParams()) { result ->
                refresh { ui.say("${d.optString("name")} is ready.") }
                if ((result as? JSONObject)?.optBoolean("ask_screen_reader") == true) askScreenReader()
            }
        }
    }

    /** A device whose Android has no screen reader. */
    private fun askScreenReader() {
        val name = device?.optString("name") ?: "The device"
        ui.choose("$name Has No Screen Reader", listOf("Install Backtalk", "Go without one")) { which ->
            if (which == 0) {
                ui.say("Downloading Backtalk.")
                call("device.screen_reader", idParams().put("choice", "backtalk")) { ui.say("$it is installed and on.") }
            } else {
                call("device.screen_reader", idParams().put("choice", "none")) {
                    ui.say("$name has no screen reader. AAE won't ask again.")
                }
            }
        }
    }

    private fun rename() {
        val d = device ?: return
        ui.ask("Rename ${d.optString("name")}", "New name", d.optString("name"), "Rename") { name ->
            if (name.isEmpty()) return@ask
            call("device.rename", idParams().put("name", name)) { refresh { ui.say("Renamed to $name.") } }
        }
    }

    private fun copy() {
        val d = device ?: return
        ui.ask("Copy ${d.optString("name")}", "Name for the copy", "${d.optString("name")} copy", "Copy") { name ->
            if (name.isEmpty()) return@ask
            ui.say("Copying.")
            call("device.copy", idParams().put("name", name)) { ui.say("Created $name.") }
        }
    }

    private fun wipe() {
        val d = device ?: return
        ui.confirm(
            "Wipe ${d.optString("name")}?",
            "Everything on it is erased, and it's set up again as new, with its screen reader. Its snapshots are deleted too.",
            "Wipe",
        ) {
            Remote.detach()
            ui.say("Wiping.")
            call("device.wipe", idParams()) { refresh { ui.say("${d.optString("name")} is wiped and ready.") } }
        }
    }

    private fun delete() {
        val d = device ?: return
        if (d.optBoolean("running")) return ui.say("Stop ${d.optString("name")} before deleting it.")
        ui.confirm(
            "Delete ${d.optString("name")}?",
            "The device and everything on it is deleted. It can't be undone. Its Android version stays installed.",
            "Delete",
        ) {
            call("device.delete", idParams()) { freed ->
                ui.message("Deleted", "Deleted ${d.optString("name")}. Freed $freed.") { finish() }
            }
        }
    }
}
