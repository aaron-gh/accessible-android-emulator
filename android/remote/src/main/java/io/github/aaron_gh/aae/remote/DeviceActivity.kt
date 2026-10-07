package io.github.aaron_gh.aae.remote

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
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
    private lateinit var microphoneButton: Button
    private lateinit var bridgeButton: Button
    private val whileRunning = mutableListOf<Button>()
    /** Starting, stopping, restarting or wiping: those buttons wait till it's done. */
    private var busy = false

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
        microphoneButton = ui.button("Turn On Microphone") { toggleMicrophone() }
        whileRunning.add(microphoneButton)
        bridgeButton = ui.button("Turn On Speech Bridge") { toggleBridge() }
        whileRunning.add(bridgeButton)
        running("Speak Status") { call("device.status", idParams()) { ui.say(it.toString()) } }
        running("Back") { press("back") }
        running("Home") { press("home") }
        running("Recent Apps") { press("recents") }
        running("Notifications") { call("device.notifications", idParams()) }
        running("Quick Settings") { call("device.quick_settings", idParams()) }
        running("Power Button") { press("power") }
        running("Assistant") { press("assistant") }
        running("Rotate Left") { call("device.rotate", idParams().put("left", true)) { ui.say(it.toString()) } }
        running("Rotate Right") { call("device.rotate", idParams().put("left", false)) { ui.say(it.toString()) } }
        running("Restart") {
            ui.say("Restarting.")
            whileBusy { done -> call("device.restart", idParams(), done) { done(); ui.say("Restarted.") } }
        }
        ui.button("Cold Boot") {
            ui.say("Cold booting.")
            whileBusy { done -> call("device.cold_boot", idParams(), done) { done(); ui.say("Ready.") } }
        }
        ui.button("Hardware") { hardware() }
        ui.button("Rename") { rename() }
        ui.button("Copy") { copy() }
        ui.button("Wipe") { wipe() }
        ui.button("Delete") { delete() }
        ui.show()
    }

    override fun onResume() {
        super.onResume()
        refresh()
        nameMicrophone()
    }

    override fun onEvent(name: String, data: JSONObject) {
        if (name == "microphone" && !data.optBoolean("on")) {
            nameMicrophone()
            ui.say(data.optString("message"))
        }
    }

    private fun nameMicrophone() {
        microphoneButton.text = if (Remote.microphoneOn) "Turn Off Microphone" else "Turn On Microphone"
    }

    /** The speech bridge: the device's speech goes to the phone's text-to-speech instead of its audio. */
    private fun toggleBridge() {
        val on = device?.optBoolean("speech_bridge") != true
        ui.say(if (on) "Turning the speech bridge on." else "Turning the speech bridge off.")
        call("device.speech_bridge", idParams().put("on", on)) { said ->
            refresh { ui.say(said.toString()) }
        }
    }

    /** The phone's microphone into the device, for voice typing or calls on it. */
    private fun toggleMicrophone() {
        if (Remote.microphoneOn) {
            Remote.setMicrophone(false)
            nameMicrophone()
            return ui.say("Microphone off.")
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            return requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), MICROPHONE_REQUEST)
        }
        try {
            Remote.setMicrophone(true)
            nameMicrophone()
            ui.say("Microphone on.")
        } catch (e: Exception) {
            ui.say(e.message ?: "The microphone couldn't be turned on.")
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != MICROPHONE_REQUEST) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) toggleMicrophone()
        else ui.say("AAE Remote isn't allowed to use the microphone.")
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
            startStop.isEnabled = !busy
            whileRunning.forEach { it.isEnabled = running && !busy }
            bridgeButton.text = if (d.optBoolean("speech_bridge")) "Turn Off Speech Bridge" else "Turn On Speech Bridge"
            if (running) attach()
            then()
        }
    }

    /** Plays the device's sound and vibrations here while it runs. */
    private fun attach() {
        Remote.attach(id, device?.optString("name") ?: "") { result ->
            result.onFailure { ui.say("The device's audio couldn't start: ${it.message}") }
        }
    }

    /** Marks the device busy while [work] runs; it calls the function it's given when done. */
    private fun whileBusy(work: (done: () -> Unit) -> Unit) {
        if (busy) return
        busy = true
        startStop.isEnabled = false
        whileRunning.forEach { it.isEnabled = false }
        work {
            busy = false
            refresh()
        }
    }

    private fun startOrStop() {
        val d = device ?: return
        if (d.optBoolean("running")) {
            Remote.detach()
            ui.say("Stopping ${d.optString("name")}.")
            whileBusy { done -> call("device.stop", idParams(), done) { done(); ui.say("Stopped.") } }
        } else {
            ui.say("Starting ${d.optString("name")}.")
            whileBusy { done -> call("device.start", idParams(), done) { result ->
                done()
                ui.say("${d.optString("name")} is ready.")
                if ((result as? JSONObject)?.optBoolean("ask_screen_reader") == true) askScreenReader()
            } }
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

    /** Memory, cores, storage and the screen, while it's stopped, from its next start. */
    private fun hardware() {
        val d = device ?: return
        if (d.optBoolean("running")) return ui.say("Stop ${d.optString("name")} to change its hardware.")
        call("device.hardware", idParams()) { result ->
            val now = result as JSONObject
            val fields = listOf(
                "memory_mb" to "Memory, in megabytes",
                "cores" to "Processor cores",
                "storage_mb" to "Storage, in megabytes",
                "width" to "Screen width, in pixels",
                "height" to "Screen height, in pixels",
                "density" to "Screen density, in dots per inch",
            )
            val column = android.widget.LinearLayout(this).apply {
                orientation = android.widget.LinearLayout.VERTICAL
                setPadding(48, 16, 48, 0)
            }
            val edits = fields.map { (key, label) ->
                val labelView = android.widget.TextView(this).apply { text = label }
                column.addView(labelView)
                android.widget.EditText(this).apply {
                    id = android.view.View.generateViewId()
                    labelView.labelFor = id
                    inputType = android.text.InputType.TYPE_CLASS_NUMBER
                    setText(now.optLong(key).toString())
                    column.addView(this)
                }
            }
            android.app.AlertDialog.Builder(this)
                .setTitle("Hardware of ${d.optString("name")}")
                .setView(android.widget.ScrollView(this).apply { addView(column) })
                .setPositiveButton("Save") { _, _ ->
                    val params = idParams()
                    for ((i, field) in fields.withIndex()) {
                        val n = edits[i].text.toString().trim().toLongOrNull() ?: return@setPositiveButton ui.say("Each one takes a whole number.")
                        params.put(field.first, n)
                    }
                    call("device.hardware.set", params) { ui.say(it.toString()) }
                }
                .setNegativeButton(android.R.string.cancel, null)
                .show()
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
            whileBusy { done -> call("device.wipe", idParams(), done) { done(); ui.say("${d.optString("name")} is wiped and ready.") } }
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

    companion object {
        private const val MICROPHONE_REQUEST = 1
    }
}
