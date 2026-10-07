package io.github.aaron_gh.aae.helper

import android.os.Binder
import android.os.IBinder
import android.os.Parcel
import android.os.SystemClock
import java.lang.reflect.Proxy

/**
 * Prints a line each time the device's vibrator turns on or off, as "on" or
 * "off" and the time since boot in milliseconds, until it's stopped. AAE's
 * Android app plays the device's vibrations on the phone from these.
 *
 * Listening to the vibrator needs the ACCESS_VIBRATOR_STATE permission, which
 * the shell user has and apps don't. There's no public way in for the shell
 * user, so this registers with the vibrator service's hidden interface, with
 * a Binder of its own standing in for the hidden IVibratorStateListener.
 */
object VibrationWatch {
    private const val LISTENER = "android.os.IVibratorStateListener"

    fun run() {
        val listener = listener()
        val registered = registerWithManager(listener) || registerWithVibrator(listener)
        if (!registered) {
            System.err.println("AAE's shell tool failed: this Android version can't report its vibrator")
            kotlin.system.exitProcess(2)
        }
        println("watching")
        System.out.flush()
        // Calls arrive on Binder threads, which app_process starts. This one
        // checks every few seconds that AAE is still reading, and ends when
        // it isn't, so no watcher is left behind on the device.
        while (true) {
            Thread.sleep(3000)
            synchronized(System.out) {
                println("alive")
                if (System.out.checkError()) kotlin.system.exitProcess(0)
            }
        }
    }

    /** A Binder that answers onVibrating(boolean), the listener's only call. */
    private class StateBinder : Binder() {
        init {
            attachInterface(null, LISTENER)
        }

        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != FIRST_CALL_TRANSACTION) return super.onTransact(code, data, reply, flags)
            data.enforceInterface(LISTENER)
            val on = data.readInt() != 0
            val time = SystemClock.elapsedRealtime()
            // What's playing, so the phone can play the same, not a buzz.
            val effect = if (on) currentEffect() else null
            synchronized(System.out) {
                println(listOfNotNull(if (on) "on" else "off", time.toString(), effect).joinToString(" "))
                System.out.flush()
            }
            return true
        }
    }

    /**
     * The effect the vibrator is playing now, as the vibrator service
     * describes it in its dump: for example "Composed{segments=[Primitive{
     * primitive=TICK, scale=0.59, delay=0}, ...]}". Read in this process, which
     * takes milliseconds, where running dumpsys would take hundreds.
     */
    private fun currentEffect(): String? = try {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, "vibrator_manager") as? IBinder
        if (binder == null) null else {
            val (read, write) = android.os.ParcelFileDescriptor.createPipe()
            // Asynchronously, so a large dump can't fill the pipe while this waits.
            binder.dumpAsync(write.fileDescriptor, arrayOf())
            write.close()
            val dump = android.os.ParcelFileDescriptor.AutoCloseInputStream(read).bufferedReader().use { it.readText() }
            val current = dump.substringAfter("CurrentVibration:", "")
            current.lineSequence().takeWhile { !it.trim().startsWith("NextVibration") }
                .firstOrNull { it.trim().startsWith("playedEffect") }
                ?.substringAfter("=")?.trim()
                ?.replace(Regex("\\s+"), " ")
        }
    } catch (_: Throwable) {
        null
    }

    /** The binder wrapped in the hidden interface, which the service's methods take. */
    private fun listener(): Any {
        val binder = StateBinder()
        val type = Class.forName(LISTENER)
        return Proxy.newProxyInstance(type.classLoader, arrayOf(type)) { _, method, _ ->
            when (method.name) {
                "asBinder" -> binder
                else -> null
            }
        }
    }

    private fun service(name: String, stub: String): Any? {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, name) as? IBinder ?: return null
        return Class.forName("$stub\$Stub").getMethod("asInterface", IBinder::class.java).invoke(null, binder)
    }

    /** Android 12 and later: each vibrator of the vibrator manager. */
    private fun registerWithManager(listener: Any): Boolean {
        val manager = try {
            service("vibrator_manager", "android.os.IVibratorManagerService")
        } catch (_: ClassNotFoundException) {
            null
        } ?: return false
        val ids = manager.javaClass.getMethod("getVibratorIds").invoke(manager) as IntArray
        val register = manager.javaClass.methods.firstOrNull {
            it.name == "registerVibratorStateListener" && it.parameterCount == 2
        } ?: return false
        return ids.map { register.invoke(manager, it, listener) as Boolean }.any { it }
    }

    /** Android 11: the single vibrator service. */
    private fun registerWithVibrator(listener: Any): Boolean {
        val vibrator = try {
            service("vibrator", "android.os.IVibratorService")
        } catch (_: ClassNotFoundException) {
            null
        } ?: return false
        val register = vibrator.javaClass.methods.firstOrNull {
            it.name == "registerVibratorStateListener" && it.parameterCount == 1
        } ?: return false
        return register.invoke(vibrator, listener) as Boolean
    }
}
