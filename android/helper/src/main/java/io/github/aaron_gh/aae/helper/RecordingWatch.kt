package io.github.aaron_gh.aae.helper

import android.os.IBinder

/**
 * Prints "recording" when any app starts recording from the microphone and
 * "quiet" when none is, until stopped. AAE injects microphone audio only
 * while Android records; at other times the emulator drops it or crashes.
 *
 * Polls IAudioService ten times a second as the shell user, which can see
 * every app's recordings.
 */
object RecordingWatch {
    fun run() {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, "audio") as IBinder
        val audio = Class.forName("android.media.IAudioService\$Stub")
            .getMethod("asInterface", IBinder::class.java)
            .invoke(null, binder)
        val active = audio.javaClass.getMethod("getActiveRecordingConfigurations")
        var recording: Boolean? = null
        var quietFor = 0
        while (true) {
            val now = (active.invoke(audio) as? List<*>)?.isNotEmpty() == true
            if (now != recording) {
                recording = now
                println(if (now) "recording" else "quiet")
                System.out.flush()
            }
            // Ends when AAE stops reading, so no watcher is left behind.
            if (++quietFor >= 30) {
                quietFor = 0
                println("alive")
                if (System.out.checkError()) kotlin.system.exitProcess(0)
            }
            Thread.sleep(100)
        }
    }
}
