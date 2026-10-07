package io.github.aaron_gh.aae.remote

import android.content.Intent
import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONArray
import org.json.JSONObject

/** The computer's devices, and what to do with them. */
class DevicesActivity : ConnectedActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private var devices = listOf<JSONObject>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Devices on ${Remote.computer?.name ?: "the computer"}")
        ui.addStatus()
        rows = ui.list("Devices") { open(devices[it]) }.second
        ui.button("New Device") { startActivity(Intent(this, NewDeviceActivity::class.java)) }
        ui.button("Android Versions") { startActivity(Intent(this, VersionsActivity::class.java)) }
        ui.button("Emulator and Tools") { startActivity(Intent(this, SetupActivity::class.java)) }
        ui.button("Refresh") { refresh() }
        ui.button("Disconnect") {
            Remote.disconnect()
            finish()
        }
        ui.show(scroll = false)
        call("server.info") { info ->
            if ((info as? JSONObject)?.optBoolean("needs_setup") == true) {
                ui.say("The computer needs Google's emulator and tools before it can run devices. Choose Emulator and Tools.")
            }
        }
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    private fun refresh() {
        call("devices.list") { result ->
            val array = result as? JSONArray ?: JSONArray()
            devices = (0 until array.length()).map { array.getJSONObject(it) }
            rows.clear()
            rows.addAll(devices.map { describe(it) })
            if (devices.isEmpty()) ui.say("No devices yet. Choose New Device.")
        }
    }

    private fun open(device: JSONObject) {
        startActivity(Intent(this, DeviceActivity::class.java).putExtra("id", device.getString("id")))
    }

    companion object {
        fun describe(d: JSONObject) =
            "${d.optString("name")}, ${d.optString("android")}, ${d.optString("kind")}, " +
                if (d.optBoolean("running")) "running" else "stopped"
    }
}
