package io.github.aaron_gh.aae.remote

import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import org.json.JSONObject

/** A testing tool's screen, for the device given as the "id" extra. */
abstract class ToolActivity : ConnectedActivity() {
    protected lateinit var id: String

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        id = intent.getStringExtra("id") ?: return finish()
    }

    /** Parameters naming the device, to add to. */
    protected fun params(): JSONObject = JSONObject().put("id", id)

    /** A file's name, as the app it came from gives it. */
    protected fun fileName(uri: Uri, otherwise: String = "app.apk"): String =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        } ?: otherwise

    /** Sends a file to the computer; [sent] gets its upload number. */
    protected fun send(uri: Uri, name: String, sent: (Int) -> Unit) {
        val connection = Remote.connection ?: return
        ui.say("Sending $name.")
        connection.call("upload.begin", JSONObject().put("name", name)) { begun ->
            val number = begun.getOrNull()?.toString()?.toIntOrNull()
                ?: return@call ui.say("The computer couldn't take the file: ${begun.exceptionOrNull()?.message}")
            Thread {
                val ok = try {
                    contentResolver.openInputStream(uri)?.use { input ->
                        val buffer = ByteArray(256 * 1024)
                        while (true) {
                            val read = input.read(buffer)
                            if (read < 0) break
                            if (!connection.uploadPart(number, buffer, read)) return@use false
                        }
                        true
                    } ?: false
                } catch (_: Exception) {
                    false
                }
                runOnUiThread {
                    if (!ok) return@runOnUiThread ui.say("$name couldn't be sent.")
                    sent(number)
                }
            }.start()
        }
    }
}

