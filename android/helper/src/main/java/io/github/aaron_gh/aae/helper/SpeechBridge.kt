package io.github.aaron_gh.aae.helper

import android.net.LocalServerSocket
import android.net.LocalSocket
import android.util.Log
import org.json.JSONObject
import java.io.IOException
import java.io.Writer
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

/**
 * Speech bridge server: while a client is connected, the speech relay sends
 * each utterance to it and blocks until the client reports it done or
 * stopped. Clients connect through adb to the abstract socket
 * "aae_speech_bridge"; only uid shell or root is accepted.
 *
 * JSON lines. Client first: {"type":"hello","polite":true}. A polite client
 * is refused while another is connected; an impolite one (aae serve)
 * replaces it. The desktop apps are polite. To the client: {"type":"speak",
 * "id":1,"text":…,"language":"en-US","rate":100,"pitch":100} and
 * {"type":"stop"}. From the client: {"type":"done","id":1}.
 */
object SpeechBridge {
    private const val SOCKET = "aae_speech_bridge"
    private const val SHELL_UID = 2000

    @Volatile private var client: LocalSocket? = null
    private var writer: Writer? = null
    private val waiting = ConcurrentHashMap<Long, CountDownLatch>()
    private val nextId = AtomicLong(1)
    private var started = false

    val connected get() = client != null

    /** Starts the server, once per process. */
    @Synchronized
    fun start() {
        if (started) return
        started = true
        Thread({ serve() }, "AAE speech bridge").apply { isDaemon = true }.start()
    }

    private fun serve() {
        val server = try {
            LocalServerSocket(SOCKET)
        } catch (e: IOException) {
            Log.w(Volume.TAG, "the speech bridge couldn't listen: $e")
            return
        }
        while (true) {
            val socket = try { server.accept() } catch (e: IOException) { continue }
            val uid = runCatching { socket.peerCredentials.uid }.getOrDefault(-1)
            if (uid != SHELL_UID && uid != 0) {
                socket.close()
                continue
            }
            val reader = socket.inputStream.bufferedReader()
            // Polite: refused while another client is connected.
            val polite = try {
                socket.soTimeout = 2000
                val hello = reader.readLine()?.let { runCatching { JSONObject(it) }.getOrNull() }
                socket.soTimeout = 0
                hello?.optBoolean("polite") == true
            } catch (_: IOException) {
                false
            }
            synchronized(this) {
                if (polite && client != null) {
                    socket.close()
                    return@synchronized
                }
                client?.close()
                client = socket
                writer = socket.outputStream.bufferedWriter()
                Thread({ read(socket, reader) }, "AAE speech bridge reader").apply { isDaemon = true }.start()
            }
        }
    }

    private fun read(socket: LocalSocket, reader: java.io.BufferedReader) {
        try {
            reader.forEachLine { line ->
                val message = runCatching { JSONObject(line) }.getOrNull() ?: return@forEachLine
                if (message.optString("type") == "done") waiting[message.optLong("id")]?.countDown()
            }
        } catch (_: IOException) {
        } finally {
            synchronized(this) {
                if (client === socket) {
                    client = null
                    writer = null
                }
            }
            // Nothing's speaking any more: let the screen reader go on.
            waiting.values.forEach { it.countDown() }
        }
    }

    /**
     * Sends an utterance to AAE. Returns a wait for it to finish, or null if
     * AAE isn't connected, when the real engine should speak instead.
     */
    fun send(text: String, language: String, rate: Int, pitch: Int): Pending? {
        val id = nextId.getAndIncrement()
        val latch = CountDownLatch(1)
        waiting[id] = latch
        val message = JSONObject()
            .put("type", "speak")
            .put("id", id)
            .put("text", text)
            .put("language", language)
            .put("rate", rate)
            .put("pitch", pitch)
        if (!write(message)) {
            waiting.remove(id)
            return null
        }
        return Pending(id, latch, text.length)
    }

    class Pending(private val id: Long, private val latch: CountDownLatch, private val length: Int) {
        /** Waits until AAE says it's done or stopped, or far longer than it could take. */
        fun await() {
            latch.await(30L + length / 5, TimeUnit.SECONDS)
            waiting.remove(id)
        }
    }

    /** The screen reader stopped speech: AAE stops, and nothing waits. */
    fun stop() {
        if (connected) write(JSONObject().put("type", "stop"))
        waiting.values.forEach { it.countDown() }
    }

    @Synchronized
    private fun write(message: JSONObject): Boolean = try {
        val out = writer ?: return false
        out.write(message.toString())
        out.write("\n")
        out.flush()
        true
    } catch (e: IOException) {
        false
    }

    /** A language tag, such as "en-US", from a request's three-letter codes. */
    fun languageTag(language: String?, country: String?): String {
        val lang = language.orEmpty()
        val two = if (lang.length == 3) {
            Locale.getAvailableLocales().firstOrNull { runCatching { it.isO3Language }.getOrNull() == lang }?.language ?: lang
        } else lang
        val region = country.orEmpty().let { c ->
            if (c.length == 3) {
                Locale.getAvailableLocales().firstOrNull { runCatching { it.isO3Country }.getOrNull() == c }?.country ?: ""
            } else c
        }
        return if (region.isEmpty()) two else "$two-$region"
    }
}
