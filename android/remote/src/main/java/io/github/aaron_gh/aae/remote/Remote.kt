package io.github.aaron_gh.aae.remote

import android.content.Context
import org.json.JSONObject

/**
 * The app's one connection to a computer, shared by every screen, and the
 * device attached to it, whose sound and vibrations play on the phone.
 */
object Remote {
    var computer: Computer? = null
        private set
    var connection: Connection? = null
        private set
    /** The attached device, and its screen size in pixels. */
    var attached: String? = null
        private set
    var screenWidth = 0
        private set
    var screenHeight = 0
        private set

    private var sound: SoundOut? = null
    private var haptics: Haptics? = null
    private val watchers = mutableListOf<Connection.Listener>()

    /** Screens that want to hear when the connection ends, or events arrive. */
    fun watch(listener: Connection.Listener) = watchers.add(listener)
    fun unwatch(listener: Connection.Listener) = watchers.remove(listener)

    val isConnected get() = connection?.isOpen == true

    private var app: Context? = null
    private var attachedName = ""
    private val main = android.os.Handler(android.os.Looper.getMainLooper())

    /** Connects to a paired computer, ending any other connection. */
    fun connect(context: Context, computer: Computer, ready: () -> Unit, failed: (String) -> Unit) {
        disconnect()
        this.computer = computer
        app = context.applicationContext
        open(computer, ready, failed)
        haptics = Haptics(context.applicationContext)
    }

    private fun open(computer: Computer, ready: () -> Unit, failed: (String) -> Unit) {
        var signedIn = false
        connection = Connection.connect(computer, object : Connection.Listener {
            override fun ready() {
                signedIn = true
                ready()
            }

            override fun closed(reason: String) {
                val device = attached
                stopSound()
                attached = null
                connection = null
                when {
                    !signedIn -> failed(reason)
                    // Wi-Fi drops for a moment, or the computer's AAE
                    // restarts: try again, and attach the same device.
                    this@Remote.computer == computer -> reconnect(computer, device, reason, attempt = 1)
                    else -> watchers.toList().forEach { it.closed(reason) }
                }
            }

            override fun event(name: String, data: JSONObject) {
                if (name == "vibration") {
                    if (data.optBoolean("on")) haptics?.on() else haptics?.off(data.optLong("ms"))
                }
                watchers.toList().forEach { it.event(name, data) }
            }
        })
    }

    private fun reconnect(computer: Computer, device: String?, reason: String, attempt: Int) {
        if (attempt > 4 || this.computer != computer) {
            stopMedia()
            watchers.toList().forEach { it.closed(reason) }
            return
        }
        main.postDelayed({
            if (this.computer != computer || connection != null) return@postDelayed
            open(computer, ready = {
                if (device != null) attach(device) {}
            }, failed = { reconnect(computer, device, reason, attempt + 1) })
        }, 2000L * attempt)
    }

    fun disconnect() {
        computer = null
        stopMedia()
        attached = null
        connection?.close()
        connection = null
    }

    /** Plays a device's sound and vibrations here, and sends it keys and touches. */
    fun attach(id: String, name: String, done: (Result<Unit>) -> Unit) {
        attachedName = name
        attach(id, done)
    }

    fun attach(id: String, done: (Result<Unit>) -> Unit) {
        val connection = connection ?: return done(Result.failure(Exception("Not connected to the computer.")))
        if (attached == id) return done(Result.success(Unit))
        connection.call("device.attach", JSONObject().put("id", id)) { result ->
            result.onSuccess { size ->
                val json = size as? JSONObject
                screenWidth = json?.optInt("width") ?: 0
                screenHeight = json?.optInt("height") ?: 0
                attached = id
                stopSound()
                val out = SoundOut()
                sound = out
                connection.sound = { bytes, offset, length -> out.write(bytes, offset, length) }
                app?.let { AttachedService.start(it, attachedName.ifEmpty { "A device" }, computer?.name ?: "the computer") }
                done(Result.success(Unit))
            }.onFailure { done(Result.failure(it)) }
        }
    }

    fun detach() {
        if (attached == null) return
        connection?.call("device.detach") {}
        attached = null
        stopSound()
    }

    private fun stopSound() {
        connection?.sound = null
        sound?.release()
        sound = null
        app?.let { AttachedService.stop(it) }
    }

    private fun stopMedia() {
        stopSound()
        haptics?.release()
    }
}
