package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONArray
import org.json.JSONObject

/** The Android versions installed on the computer, and deleting them. */
class VersionsActivity : ConnectedActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private var images = listOf<JSONObject>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Android Versions")
        ui.text("A version can be deleted once none of AAE's devices use it. Choose one to delete it.")
        ui.addStatus()
        rows = ui.list("Installed Android versions") { delete(images[it]) }.second
        ui.button("Refresh") { refresh() }
        ui.show(scroll = false)
        refresh()
    }

    private fun refresh() {
        call("images.installed") { result ->
            val array = result as? JSONArray ?: JSONArray()
            images = (0 until array.length()).map { array.getJSONObject(it) }
            rows.clear()
            rows.addAll(images.map { i ->
                val devices = i.optJSONArray("devices")
                val used = if (devices == null || devices.length() == 0) "No AAE devices use it"
                else "Used by " + (0 until devices.length()).joinToString(", ") { devices.getString(it) }
                "${i.optString("description")}. ${i.optString("size")}. $used."
            })
            if (images.isEmpty()) ui.say("No Android versions are installed.")
        }
    }

    private fun delete(image: JSONObject) {
        val devices = image.optJSONArray("devices")
        if (devices != null && devices.length() > 0) {
            return ui.say("${image.optString("description")} can't be deleted while devices use it. Delete those devices first.")
        }
        ui.confirm(
            "Delete ${image.optString("description")}?",
            "This deletes its ${image.optString("size")} of files from the computer. It can be downloaded again later.",
            "Delete",
        ) {
            ui.say("Deleting.")
            call("images.remove", JSONObject().put("sysdir", image.optString("sysdir"))) { freed ->
                ui.say("Deleted ${image.optString("description")}. Freed $freed.")
                refresh()
            }
        }
    }
}
