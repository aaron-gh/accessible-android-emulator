package io.github.aaron_gh.aae.remote

import android.app.AlertDialog
import android.os.Bundle
import android.widget.ArrayAdapter
import android.widget.CheckBox
import org.json.JSONArray
import org.json.JSONObject

/** The device's apps, and opening, stopping, clearing and uninstalling them, and their permissions. */
class AppsActivity : ToolActivity() {
    private lateinit var rows: ArrayAdapter<String>
    private lateinit var system: CheckBox
    private var apps = listOf<JSONObject>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Apps")
        system = CheckBox(this).apply {
            text = "Show Android's own apps"
            setOnClickListener { load() }
            ui.column.addView(this)
        }
        ui.addStatus()
        rows = ui.list("Apps") { act(apps[it]) }.second
        ui.button("Refresh") { load() }
        ui.show(scroll = false)
        load()
    }

    private fun load() {
        ui.say("Reading.")
        call("tools.apps", params().put("system", system.isChecked)) { result ->
            val array = result as? JSONArray ?: JSONArray()
            apps = (0 until array.length()).map { array.getJSONObject(it) }
            rows.clear()
            rows.addAll(apps.map { a ->
                listOfNotNull(
                    a.optString("label"), a.optString("package"),
                    a.optString("version").takeIf { it.isNotEmpty() }?.let { "version $it" },
                    "turned off".takeIf { !a.optBoolean("enabled") },
                ).joinToString(", ")
            })
            ui.say(if (apps.isEmpty()) "No apps you've installed. Show Android's own apps to see the rest." else "${apps.size} apps.")
        }
    }

    private fun act(app: JSONObject) {
        val label = app.optString("label")
        val pkg = params().put("package", app.optString("package"))
        val actions = mutableListOf<Pair<String, () -> Unit>>()
        if (app.optBoolean("launchable")) actions += "Open" to { call("tools.app.open", pkg) { ui.say("Opened $label.") } }
        actions += "Force Stop" to { call("tools.app.stop", pkg) { ui.say("Stopped $label.") } }
        actions += "Permissions" to { permissions(app) }
        actions += "Clear Data" to {
            ui.confirm("Clear $label's data?", "It goes back to how it was when first installed: signed out, with its settings and files deleted.", "Clear Data") {
                call("tools.app.clear", pkg) { ui.say("Cleared $label's data.") }
            }
        }
        if (!app.optBoolean("system")) actions += "Uninstall" to {
            ui.confirm("Uninstall $label?", "It and its data are removed from the device.", "Uninstall") {
                call("tools.app.uninstall", pkg) { ui.say("Uninstalled $label."); load() }
            }
        }
        ui.choose(label, actions.map { it.first }) { actions[it].second() }
    }

    /** Permissions and special access, each a checkbox that changes it at once. */
    private fun permissions(app: JSONObject) {
        val pkg = app.optString("package")
        call("tools.app.permissions", params().put("package", pkg)) { result ->
            val json = result as JSONObject
            val permissions = json.optJSONArray("permissions") ?: JSONArray()
            val access = json.optJSONArray("access") ?: JSONArray()
            val labels = (0 until permissions.length()).map { permissions.getJSONObject(it).optString("label") } +
                (0 until access.length()).map { "Special access: " + access.getJSONObject(it).optString("name") }
            val checked = ((0 until permissions.length()).map { permissions.getJSONObject(it).optBoolean("granted") } +
                (0 until access.length()).map { access.getJSONObject(it).optBoolean("allowed") }).toBooleanArray()
            AlertDialog.Builder(this)
                .setTitle("Permissions of ${app.optString("label")}")
                .setMultiChoiceItems(labels.toTypedArray(), checked) { _, which, on ->
                    if (which < permissions.length()) {
                        val p = permissions.getJSONObject(which)
                        call("tools.app.permission", params().put("package", pkg).put("permission", p.optString("name")).put("granted", on)) {
                            ui.say("${p.optString("label")}: ${if (on) "granted" else "revoked"}.")
                        }
                    } else {
                        val a = access.getJSONObject(which - permissions.length())
                        call("tools.app.access", params().put("package", pkg).put("kind", a.optString("kind")).put("allowed", on)) {
                            ui.say("${a.optString("name")}: ${if (on) "allowed" else "not allowed"}.")
                        }
                    }
                }
                .setNeutralButton("Grant All") { _, _ ->
                    call("tools.app.grant_all", params().put("package", pkg)) { n -> ui.say("Granted $n permissions.") }
                }
                .setPositiveButton("Done", null)
                .show()
        }
    }
}
