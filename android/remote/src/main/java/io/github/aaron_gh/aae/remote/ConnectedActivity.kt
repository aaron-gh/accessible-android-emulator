package io.github.aaron_gh.aae.remote

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import org.json.JSONObject

/**
 * A screen that needs the connection to the computer. If it ends, the screen
 * says why and goes back to the computers.
 */
abstract class ConnectedActivity : Activity() {
    protected lateinit var ui: Ui

    private val watcher = object : Connection.Listener {
        override fun closed(reason: String) = lost(reason)
        override fun event(name: String, data: JSONObject) = onEvent(name, data)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = Ui(this)
        if (Remote.connection == null) {
            finish()
            return
        }
        Remote.watch(watcher)
    }

    override fun onDestroy() {
        Remote.unwatch(watcher)
        super.onDestroy()
    }

    open fun onEvent(name: String, data: JSONObject) {}

    private fun lost(reason: String) {
        if (isFinishing) return
        ui.message("Disconnected", "The connection to the computer ended. $reason") {
            startActivity(
                Intent(this, ComputersActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            )
            finish()
        }
    }

    /** Calls the computer, saying progress and any failure in the status line. */
    protected fun call(
        method: String,
        params: JSONObject = JSONObject(),
        failed: () -> Unit = {},
        done: (Any?) -> Unit = {},
    ) {
        val connection = Remote.connection ?: run {
            ui.say("Not connected to the computer.")
            failed()
            return
        }
        connection.call(method, params, progress = { message, percent ->
            when {
                message != null -> ui.say(message)
                percent != null && percent % 10 == 0 -> ui.say("$percent percent.")
            }
        }) { result ->
            result.onSuccess(done).onFailure {
                ui.say(it.message ?: "That didn't work.")
                failed()
            }
        }
    }
}
