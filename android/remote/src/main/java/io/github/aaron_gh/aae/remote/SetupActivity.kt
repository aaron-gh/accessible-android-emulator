package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/** Google's emulator and tools on the computer: setting them up, and updating them. */
class SetupActivity : ConnectedActivity() {
    private lateinit var details: TextView
    private lateinit var action: Button
    private var update = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Emulator and Tools")
        details = ui.text("Checking.")
        ui.addStatus()
        action = ui.button("Download and Set Up") { install() }
        action.isEnabled = false
        ui.show()
        check()
    }

    private fun names(array: JSONArray?) =
        (0 until (array?.length() ?: 0)).joinToString("; ") {
            val tool = array!!.getJSONObject(it)
            "${tool.optString("name")} ${tool.optString("revision")}, ${tool.optString("size")}"
        }

    private fun check() {
        call("setup.status", JSONObject().put("refresh", true)) { result ->
            val status = result as JSONObject
            val missing = status.optJSONArray("missing")
            val updates = status.optJSONArray("updates")
            val lines = mutableListOf<String>()
            status.optString("virtualisation_problem").takeIf { it.isNotEmpty() && it != "null" }?.let { lines += it }
            status.optString("performance_warning").takeIf { it.isNotEmpty() && it != "null" }?.let { lines += it }
            when {
                (missing?.length() ?: 0) > 0 -> {
                    lines += "The computer needs these, ${status.optString("missing_size")} in all: ${names(missing)}."
                    update = false
                    action.text = "Download and Set Up"
                    action.isEnabled = status.isNull("virtualisation_problem")
                }
                (updates?.length() ?: 0) > 0 -> {
                    lines += "Updates: ${names(updates)}. Stop every device before updating."
                    update = true
                    action.text = "Update the Emulator and Tools"
                    action.isEnabled = true
                }
                else -> {
                    lines += "The emulator and tools are set up and up to date."
                    action.isEnabled = false
                }
            }
            lines += "They're in ${status.optString("sdk_path")} on the computer."
            details.text = lines.joinToString("\n\n")
        }
    }

    private fun install() {
        call("setup.licence", JSONObject().put("update", update)) { licence ->
            val json = licence as? JSONObject
            if (json == null) {
                download()
            } else {
                ui.licence("Google's Licence", json.optString("text")) { accepted ->
                    if (!accepted) return@licence ui.say("Licence declined, so nothing was downloaded.")
                    call("licence.accept", JSONObject().put("licence", json.optString("id")).put("text", json.optString("text"))) {
                        download()
                    }
                }
            }
        }
    }

    private fun download() {
        action.isEnabled = false
        ui.say("Downloading.")
        call("setup.install", JSONObject().put("update", update)) {
            ui.say("The emulator and tools are ready.")
            check()
        }
    }
}
