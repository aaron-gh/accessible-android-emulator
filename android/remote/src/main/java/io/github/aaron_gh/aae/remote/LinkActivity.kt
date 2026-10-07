package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.widget.CheckBox
import org.json.JSONObject

/** Opens a link on the device, or sends it an intent, to test how apps answer them. */
class LinkActivity : ToolActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Open a Link")
        val link = ui.field("Link, such as https://example.com or myapp://settings")
        ui.addStatus()
        ui.button("Open") {
            val text = link.text.toString().trim()
            if (text.isEmpty()) return@button
            call("tools.link", params().put("link", text)) { ui.say("Opened $text.") }
        }

        ui.heading("Send an Intent")
        val action = ui.field("Action, such as android.intent.action.VIEW")
        val data = ui.field("Data, such as a link")
        val target = ui.field("To: a package, or package/class for a screen or receiver")
        val extras = ui.field("Text extras, as key=value, separated by semicolons")
        val broadcast = CheckBox(this).apply { text = "Send as a broadcast, not to open a screen"; ui.column.addView(this) }
        ui.button("Send") {
            val pairs = JSONObject()
            extras.text.toString().split(';').forEach { part ->
                val (key, value) = part.split('=', limit = 2).takeIf { it.size == 2 } ?: return@forEach
                pairs.put(key.trim(), value)
            }
            val p = params().put("action", action.text.toString()).put("data", data.text.toString())
                .put("target", target.text.toString()).put("extras", pairs).put("broadcast", broadcast.isChecked)
            call("tools.intent", p) { said -> ui.say(said?.toString()?.ifEmpty { "Sent." } ?: "Sent.") }
        }
        ui.show()
    }
}
