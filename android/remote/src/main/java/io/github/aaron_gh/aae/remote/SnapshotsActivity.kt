package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.ArrayAdapter
import org.json.JSONArray
import org.json.JSONObject

/** Saved states of the device to come back to, such as before testing a sign-in. */
class SnapshotsActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private var snapshots = listOf<JSONObject>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Snapshots")
        ui.text("A snapshot saves everything on the device, so you can come back to it later.")
        ui.addStatus()
        rows = ui.list("Snapshots") { act(snapshots[it]) }.second
        ui.button("Save Snapshot") {
            ui.ask("Save a Snapshot", "Name", action = "Save") { name ->
                if (name.isEmpty()) return@ask
                ui.say("Saving snapshot $name.")
                call("tools.snapshot.save", params().put("name", name)) { ui.say("Saved snapshot $name."); load() }
            }
        }
        ui.show(scroll = false)
        load()
    }

    private fun load() {
        call("tools.snapshots", params()) { result ->
            val array = result as? JSONArray ?: JSONArray()
            snapshots = (0 until array.length()).map { array.getJSONObject(it) }
            rows.clear()
            rows.addAll(snapshots.map { s ->
                listOfNotNull(
                    s.optString("name"),
                    s.optString("taken").takeIf { it.isNotEmpty() && it != "null" }?.let { "taken $it" },
                    s.optString("size"),
                    "restored last".takeIf { s.optBoolean("loaded") },
                    "this emulator can't restore it".takeIf { !s.optBoolean("compatible") },
                ).joinToString(", ")
            })
            if (snapshots.isEmpty()) ui.say("No snapshots yet.")
        }
    }

    private fun act(snapshot: JSONObject) {
        val name = snapshot.optString("name")
        val p = params().put("snapshot", snapshot.optString("id"))
        ui.choose(name, listOf("Restore", "Delete")) { which ->
            if (which == 0) {
                ui.confirm("Restore $name?", "The device goes back to how it was when this snapshot was taken. Anything since then is lost, unless you save a snapshot of it first.", "Restore") {
                    ui.say("Restoring $name.")
                    call("tools.snapshot.load", p) { ui.say("Restored $name."); load() }
                }
            } else {
                ui.confirm("Delete $name?", "It can't be undone.", "Delete") {
                    call("tools.snapshot.delete", p) { ui.say("Deleted $name."); load() }
                }
            }
        }
    }
}
