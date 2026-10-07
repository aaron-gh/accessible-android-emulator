package io.github.aaron_gh.aae.remote

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.widget.ArrayAdapter

/**
 * The first screen: the computers this phone has paired with, and pairing
 * with another. Choosing a computer connects to it.
 */
class ComputersActivity : Activity() {
    private lateinit var ui: Ui
    private lateinit var rows: ArrayAdapter<String>
    private var computers = listOf<Computer>()
    private var found = listOf<Found>()
    private lateinit var discovery: Discovery

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = Ui(this)
        ui.heading("Computers")
        ui.text("Choose a computer running AAE to use its devices. On the computer, AAE must be serving: run aae serve, or turn on serving in the AAE app.")
        ui.addStatus()
        rows = ui.list("Computers") { open(computers[it]) }.second
        ui.button("Pair with a Computer") { pair() }
        ui.button("Forget a Computer") { forget() }
        ui.show(scroll = false)
        discovery = Discovery(this) { list ->
            found = list
            // A paired computer that's moved gets its new address.
            list.forEach { Computers.moved(this, it.fingerprint, it.host, it.port) }
            refresh()
        }
    }

    override fun onResume() {
        super.onResume()
        refresh()
        discovery.start()
    }

    override fun onPause() {
        super.onPause()
        discovery.stop()
    }

    private fun refresh() {
        computers = Computers.list(this)
        rows.clear()
        rows.addAll(computers.map { c ->
            val here = found.any { it.fingerprint == c.fingerprint }
            "${c.name}${if (here) ", on this network" else ""}"
        })
        if (computers.isEmpty()) ui.say("No computers are paired yet. Choose Pair with a Computer.")
    }

    private fun open(computer: Computer) {
        ui.say("Connecting to ${computer.name}.")
        Remote.connect(this, computer, ready = {
            ui.say("Connected to ${computer.name}.")
            startActivity(Intent(this, DevicesActivity::class.java))
        }, failed = { ui.say(it) })
    }

    private fun pair() {
        val paired = computers.map { it.fingerprint }.toSet()
        val choices = found.filter { it.fingerprint !in paired }
        val items = choices.map { "${it.name}, found on this network" } + "Type the computer's address"
        ui.choose("Pair with Which Computer?", items) { which ->
            if (which < choices.size) {
                askCode(choices[which].host, choices[which].port)
            } else {
                ui.ask("Computer's Address", "Address, such as 192.168.1.20", action = "Next") { text ->
                    val (host, port) = parseAddress(text) ?: return@ask ui.say("That isn't an address.")
                    askCode(host, port)
                }
            }
        }
    }

    private fun askCode(host: String, port: Int) {
        ui.ask("Pairing Code", "The code AAE shows on the computer", action = "Pair") { code ->
            if (Connection.normaliseCode(code).length != 12) {
                return@ask ui.say("The code has twelve letters and numbers. Check it on the computer, and try again.")
            }
            ui.say("Pairing.")
            Connection.pair(host, port, code, phoneName()) { result ->
                runOnUiThread {
                    result.onSuccess {
                        Computers.add(this, it)
                        refresh()
                        ui.say("Paired with ${it.name}.")
                        open(it)
                    }.onFailure { ui.say(it.message ?: "Pairing failed.") }
                }
            }
        }
    }

    private fun forget() {
        if (computers.isEmpty()) return ui.say("No computers are paired.")
        ui.choose("Forget Which Computer?", computers.map { it.name }) { which ->
            val computer = computers[which]
            ui.confirm(
                "Forget ${computer.name}?",
                "The phone won't connect to it until you pair again. On the computer, aae phones lists the phones paired with it.",
                "Forget",
            ) {
                if (Remote.computer?.fingerprint == computer.fingerprint) Remote.disconnect()
                Computers.remove(this, computer)
                refresh()
                ui.say("Forgot ${computer.name}.")
            }
        }
    }

    /** The phone's name, as the computer lists it. */
    private fun phoneName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
            ?: "${Build.MANUFACTURER} ${Build.MODEL}"

    companion object {
        /** "host", "host:port", or "[ipv6]:port". */
        fun parseAddress(text: String): Pair<String, Int>? {
            val t = text.trim()
            if (t.isEmpty()) return null
            if (t.startsWith("[")) {
                val end = t.indexOf(']')
                if (end < 0) return null
                val port = t.substring(end + 1).removePrefix(":").toIntOrNull() ?: Connection.DEFAULT_PORT
                return t.substring(1, end) to port
            }
            val parts = t.split(":")
            return when (parts.size) {
                1 -> t to Connection.DEFAULT_PORT
                2 -> parts[0] to (parts[1].toIntOrNull() ?: return null)
                else -> t to Connection.DEFAULT_PORT
            }
        }
    }
}
