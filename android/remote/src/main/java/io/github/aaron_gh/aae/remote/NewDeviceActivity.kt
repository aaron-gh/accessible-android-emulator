package io.github.aaron_gh.aae.remote

import android.content.Intent
import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.CheckBox
import android.widget.EditText
import android.widget.Spinner
import org.json.JSONArray
import org.json.JSONObject

/**
 * Makes a device: chooses an Android version, downloading it after Google's
 * licence if the computer hasn't got it, then creates and starts the device.
 */
class NewDeviceActivity : ConnectedActivity() {
    private lateinit var name: EditText
    private lateinit var version: Spinner
    private lateinit var versions: ArrayAdapter<String>
    private lateinit var previews: CheckBox
    private lateinit var profile: Spinner
    private lateinit var createButton: android.widget.Button

    /** While a device is being made, Create and the choices wait. */
    private fun setBusy(busy: Boolean) {
        createButton.isEnabled = !busy
        name.isEnabled = !busy
        version.isEnabled = !busy
        previews.isEnabled = !busy
        profile.isEnabled = !busy
    }
    private var list = listOf<JSONObject>()
    private val profiles = listOf("phone" to "Phone", "small-phone" to "Small phone", "tablet" to "Tablet", "foldable" to "Foldable")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("New Device")
        name = ui.field("Name")
        val versionLabel = ui.text("Android version")
        versions = ArrayAdapter<String>(this, android.R.layout.simple_spinner_item).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        version = Spinner(this).apply {
            id = android.view.View.generateViewId()
            versionLabel.labelFor = id
            adapter = versions
            ui.column.addView(this)
        }
        previews = CheckBox(this).apply {
            text = "Include previews of upcoming Android releases"
            setOnCheckedChangeListener { _, _ -> load() }
            ui.column.addView(this)
        }
        val profileLabel = ui.text("Kind of device")
        profile = Spinner(this).apply {
            id = android.view.View.generateViewId()
            profileLabel.labelFor = id
            adapter = ArrayAdapter(this@NewDeviceActivity, android.R.layout.simple_spinner_item, profiles.map { it.second }).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            ui.column.addView(this)
        }
        ui.addStatus()
        createButton = ui.button("Create") { create() }
        ui.show()
        load()
    }

    private fun load() {
        ui.say("Reading the Android versions.")
        call("versions.list", JSONObject().put("previews", previews.isChecked)) { result ->
            val array = result as? JSONArray ?: JSONArray()
            // Installed versions first, ready to use; then the rest, newest first.
            list = (0 until array.length()).map { array.getJSONObject(it) }
                .sortedBy { !it.optBoolean("installed") }
            versions.clear()
            versions.addAll(list.map {
                val where = if (it.optBoolean("installed")) "installed" else "${it.optString("size")} to download"
                "${it.optString("description")}, $where"
            })
            ui.say("${list.size} Android versions.")
        }
    }

    private fun create() {
        if (!createButton.isEnabled) return
        val chosen = list.getOrNull(version.selectedItemPosition) ?: return ui.say("Choose an Android version.")
        setBusy(true)
        val failed = { setBusy(false) }
        val deviceName = name.text.toString().trim().ifEmpty { chosen.optString("description") }
        if (chosen.optBoolean("installed")) {
            make(deviceName, chosen.optString("sysdir"))
            return
        }
        // Google's licence, which the person here reads and accepts.
        call("versions.licence", JSONObject().put("id", chosen.optString("id")), failed) { licence ->
            val json = licence as? JSONObject
            if (json == null) {
                download(chosen, deviceName)
            } else {
                ui.licence("Google's Licence for ${chosen.optString("description")}", json.optString("text")) { accepted ->
                    if (!accepted) {
                        setBusy(false)
                        return@licence ui.say("Licence declined, so nothing was downloaded.")
                    }
                    call("licence.accept", JSONObject().put("licence", json.optString("id")).put("text", json.optString("text")), failed) {
                        download(chosen, deviceName)
                    }
                }
            }
        }
    }

    private fun download(version: JSONObject, deviceName: String) {
        ui.say("Downloading ${version.optString("description")}, ${version.optString("size")}.")
        call("versions.install", JSONObject().put("id", version.optString("id")), { setBusy(false) }) { image ->
            ui.say("${version.optString("description")} is installed.")
            make(deviceName, (image as JSONObject).optString("sysdir"))
        }
    }

    private fun make(deviceName: String, sysdir: String) {
        val kind = profiles.getOrNull(profile.selectedItemPosition)?.first ?: "phone"
        call("devices.create", JSONObject().put("name", deviceName).put("sysdir", sysdir).put("profile", kind), { setBusy(false) }) { made ->
            val id = (made as JSONObject).optString("id")
            ui.say("Created $deviceName. Open it to start it.")
            startActivity(Intent(this, DeviceActivity::class.java).putExtra("id", id))
            finish()
        }
    }
}
