package io.github.aaron_gh.aae.remote

import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import org.json.JSONArray
import org.json.JSONObject

/** The device's testing tools, the clipboard, and installing apps from the phone. */
class ToolsActivity : ToolActivity() {
    private lateinit var playButton: android.widget.Button
    /** A sound file is waiting for, or playing into, the device's microphone. */
    private var playing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        ui.heading("Testing Tools")
        ui.addStatus()
        fun open(label: String, screen: Class<*>) =
            ui.button(label) { startActivity(Intent(this, screen).putExtra("id", id)) }
        open("Speech Log", SpeechLogActivity::class.java)
        open("Device Log", LogActivity::class.java)
        open("Accessibility Inspector", InspectorActivity::class.java)
        open("Apps", AppsActivity::class.java)
        open("Accessibility Services", ServicesActivity::class.java)
        open("Snapshots", SnapshotsActivity::class.java)
        open("Battery, Location, Phone and Network", ConditionsActivity::class.java)
        open("Display and Language", DeviceSettingsActivity::class.java)
        open("Open Link or Send Intent", LinkActivity::class.java)
        ui.button("Install App") { chooseApk() }
        playButton = ui.button("Play Audio File into Microphone") { playOrStop() }
        ui.button("Send Phone Clipboard to Device") { sendClipboard(type = false) }
        ui.button("Type Phone Clipboard on Device") { sendClipboard(type = true) }
        ui.button("Copy Device Clipboard to Phone") { copyClipboard() }
        ui.button("Read the Screen's Text") {
            call("tools.screen_text", params()) { text ->
                ui.message("The Screen's Text", text?.toString()?.ifEmpty { "The screen has no text." } ?: "")
            }
        }
        ui.show()
    }

    private fun clipboard() = getSystemService(ClipboardManager::class.java)

    private fun sendClipboard(type: Boolean) {
        val text = clipboard().primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()
        if (text.isNullOrEmpty()) return ui.say("The phone's clipboard has no text.")
        call(if (type) "tools.type" else "tools.clipboard.set", params().put("text", text)) {
            ui.say(if (type) "Typed it on the device." else "It's on the device's clipboard.")
        }
    }

    private fun copyClipboard() {
        call("tools.clipboard", params()) { text ->
            val value = text?.toString().orEmpty()
            if (value.isEmpty()) return@call ui.say("The device's clipboard is empty.")
            clipboard().setPrimaryClip(ClipData.newPlainText("From the device", value))
            ui.say("Copied from the device.")
        }
    }

    private fun chooseApk() {
        val pick = Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
            .setType("application/vnd.android.package-archive")
        startActivityForResult(pick, CHOOSE_APK)
    }

    /** A sound file into the device's microphone, for testing voice input; or stops one. */
    private fun playOrStop() {
        if (playing) {
            call("tools.microphone.stop", params())
            return
        }
        val pick = Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("audio/*")
        startActivityForResult(pick, CHOOSE_SOUND)
    }

    private fun play(uri: Uri) {
        val name = fileName(uri, "sound")
        send(uri, name) { number ->
            playing = true
            playButton.text = "Stop Playing Audio File"
            ui.say("$name queued for the next recording.")
            call("tools.microphone.play", params().put("upload", number), failed = { stopped() }) { said ->
                stopped()
                ui.say(said.toString())
            }
        }
    }

    private fun stopped() {
        playing = false
        playButton.text = "Play Audio File into Microphone"
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        when (requestCode) {
            CHOOSE_APK -> install(uri)
            CHOOSE_SOUND -> play(uri)
        }
    }

    /** Sends the APK to the computer, which installs it on the device. */
    private fun install(uri: Uri) {
        val name = fileName(uri)
        send(uri, name) { number ->
            ui.say("Installing $name.")
            call("tools.install", params().put("upload", number)) { result ->
                val json = result as JSONObject
                ui.say("Installed ${json.optString("package")}.")
                askParts(json.optJSONArray("parts") ?: JSONArray())
            }
        }
    }

    /** Asks which of the app's special parts to turn on, such as an accessibility service. */
    private fun askParts(parts: JSONArray) {
        val list = (0 until parts.length()).map { parts.getJSONObject(it) }.filter { it.isNull("choice") }
        if (list.isEmpty()) return
        val on = BooleanArray(list.size) { false }
        AlertDialog.Builder(this)
            .setTitle("Turn These On?")
            .setMultiChoiceItems(list.map { "${it.optString("name")}, ${it.optString("kind")}" }.toTypedArray(), on) { _, which, checked ->
                on[which] = checked
            }
            .setPositiveButton("Done") { _, _ ->
                val choices = JSONArray()
                list.forEachIndexed { i, p -> choices.put(JSONObject(p.toString()).put("on", on[i])) }
                call("tools.app.choices", params().put("choices", choices)) { ui.say("Done.") }
            }
            .show()
    }

    companion object {
        private const val CHOOSE_APK = 1
        private const val CHOOSE_SOUND = 2
    }
}
