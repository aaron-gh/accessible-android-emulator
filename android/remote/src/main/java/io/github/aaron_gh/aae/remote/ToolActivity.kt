package io.github.aaron_gh.aae.remote

import android.os.Bundle
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
}
