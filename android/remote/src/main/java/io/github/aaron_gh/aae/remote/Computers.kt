package io.github.aaron_gh.aae.remote

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** A computer running AAE that this phone has paired with. */
data class Computer(
    val name: String,
    val host: String,
    val port: Int,
    /** The SHA-256 of its certificate, as lowercase hex: anything else is refused. */
    val fingerprint: String,
    val client: String,
    val token: String,
) {
    fun toJson(): JSONObject = JSONObject()
        .put("name", name).put("host", host).put("port", port)
        .put("fingerprint", fingerprint).put("client", client).put("token", token)

    companion object {
        fun fromJson(json: JSONObject) = Computer(
            json.getString("name"), json.getString("host"), json.getInt("port"),
            json.getString("fingerprint"), json.getString("client"), json.getString("token"),
        )
    }
}

/** The paired computers, kept in the app's private preferences. */
object Computers {
    private const val KEY = "computers"

    private fun prefs(context: Context) = context.getSharedPreferences("aae", Context.MODE_PRIVATE)

    fun list(context: Context): List<Computer> {
        val array = JSONArray(prefs(context).getString(KEY, "[]"))
        return (0 until array.length()).map { Computer.fromJson(array.getJSONObject(it)) }
    }

    private fun save(context: Context, list: List<Computer>) {
        val array = JSONArray()
        list.forEach { array.put(it.toJson()) }
        prefs(context).edit().putString(KEY, array.toString()).apply()
    }

    /** Adds a computer, replacing one with the same certificate. */
    fun add(context: Context, computer: Computer) {
        save(context, list(context).filter { it.fingerprint != computer.fingerprint } + computer)
    }

    fun remove(context: Context, computer: Computer) {
        save(context, list(context).filter { it.fingerprint != computer.fingerprint })
    }

    /** A computer found on the network may have a new address: remembers it. */
    fun moved(context: Context, fingerprint: String, host: String, port: Int) {
        save(context, list(context).map {
            if (it.fingerprint == fingerprint) it.copy(host = host, port = port) else it
        })
    }
}
