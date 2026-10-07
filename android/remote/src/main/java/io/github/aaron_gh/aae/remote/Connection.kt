package io.github.aaron_gh.aae.remote

import android.os.Handler
import android.os.Looper
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

/**
 * A connection to AAE on a computer: TLS with the computer's own certificate,
 * pinned by its fingerprint once paired, and a WebSocket carrying JSON
 * messages and the attached device's sound. See aae-remote's server.rs for
 * the messages.
 *
 * Callbacks come on the main thread, except [sound], which comes on
 * OkHttp's thread so it isn't held up.
 */
class Connection private constructor(
    private val host: String,
    private val port: Int,
    /** The fingerprint to insist on, or null when pairing. */
    expected: String?,
) {
    interface Listener {
        /** Signed in, or paired. */
        fun ready() {}
        /** The connection ended, with why, in words. */
        fun closed(reason: String) {}
        /** An event: "vibration" and so on. */
        fun event(name: String, data: JSONObject) {}
    }

    private val main = Handler(Looper.getMainLooper())
    private val trust = PinningTrust(expected)
    private var socket: WebSocket? = null
    private var nextId = 1
    private val calls = HashMap<Int, Call>()
    var listener: Listener? = null
    /** The attached device's sound: 16-bit stereo samples at 48 kHz. */
    var sound: ((ByteArray, Int, Int) -> Unit)? = null
    @Volatile var isOpen = false
        private set

    private class Call(val progress: ((String?, Int?) -> Unit)?, val done: (Result<Any?>) -> Unit)

    /** Accepts the certificate it was told to expect, or, when pairing, any, noting which. */
    private class PinningTrust(private val expected: String?) : X509TrustManager {
        @Volatile var seen: String? = null

        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String) {
            val fingerprint = sha256(chain[0].encoded)
            seen = fingerprint
            if (expected != null && fingerprint != expected) {
                throw CertificateException("This computer's certificate has changed")
            }
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String) {
            throw CertificateException()
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    private fun open(first: (WebSocket) -> Unit) {
        val tls = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trust), SecureRandom()) }
        val client = OkHttpClient.Builder()
            .sslSocketFactory(tls.socketFactory, trust)
            // The certificate is pinned, so the name in it doesn't matter.
            .hostnameVerifier { _, _ -> true }
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.SECONDS)
            .pingInterval(15, TimeUnit.SECONDS)
            .build()
        val request = Request.Builder().url("https://$host:$port/").build()
        socket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) = first(webSocket)

            override fun onMessage(webSocket: WebSocket, text: String) {
                val message = try {
                    JSONObject(text)
                } catch (_: Exception) {
                    return
                }
                main.post { received(message) }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                val data = bytes.toByteArray()
                if (data.isNotEmpty() && data[0] == AUDIO_FRAME) sound?.invoke(data, 1, data.size - 1)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = ended(reason)

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val cause = generateSequence(t) { it.cause }.firstOrNull { it is CertificateException }
                ended(
                    when {
                        cause != null -> "This computer's certificate has changed, so the phone didn't connect. If AAE was set up again on it, pair again."
                        else -> "Couldn't reach $host: ${t.message ?: t.javaClass.simpleName}"
                    }
                )
            }
        })
    }

    private fun ended(reason: String) {
        main.post {
            if (!isOpen && calls.isEmpty() && listener == null) return@post
            isOpen = false
            val pending = calls.values.toList()
            calls.clear()
            pending.forEach { it.done(Result.failure(Exception(reason))) }
            listener?.closed(reason)
        }
    }

    private fun received(message: JSONObject) {
        when (message.optString("type")) {
            "welcome" -> {
                isOpen = true
                listener?.ready()
            }
            "denied" -> {
                isOpen = false
                listener?.closed(message.optString("message"))
                socket?.close(1000, null)
            }
            "result" -> calls.remove(message.optInt("id"))?.done?.invoke(Result.success(message.opt("result")))
            "error" -> calls.remove(message.optInt("id"))
                ?.done?.invoke(Result.failure(Exception(message.optString("message"))))
            "progress" -> calls[message.optInt("id")]?.progress?.invoke(
                message.optString("message").takeIf { it.isNotEmpty() },
                if (message.has("percent")) message.optInt("percent") else null,
            )
            "event" -> listener?.event(message.optString("event"), message)
            "paired" -> pairedReply?.invoke(message)
        }
    }

    private var pairedReply: ((JSONObject) -> Unit)? = null

    /** Calls a method of AAE on the computer. [progress] hears messages and percentages on the way. */
    fun call(
        method: String,
        params: JSONObject = JSONObject(),
        progress: ((String?, Int?) -> Unit)? = null,
        done: (Result<Any?>) -> Unit,
    ) {
        val id = nextId++
        calls[id] = Call(progress, done)
        val sent = socket?.send(
            JSONObject().put("type", "call").put("id", id).put("method", method).put("params", params).toString()
        ) ?: false
        if (!sent) calls.remove(id)?.done?.invoke(Result.failure(Exception("Not connected to the computer.")))
    }

    /** A key on the attached device, by its Linux key code (an Android key event's scan code). */
    fun key(code: Int, down: Boolean) {
        socket?.send(JSONObject().put("type", "key").put("code", code).put("down", down).toString())
    }

    /** Fingers on the attached device's screen, in its pixels. */
    fun touch(points: JSONArray) {
        socket?.send(JSONObject().put("type", "touch").put("points", points).toString())
    }

    fun close() {
        listener = null
        sound = null
        isOpen = false
        socket?.close(1000, null)
        socket = null
    }

    companion object {
        const val AUDIO_FRAME: Byte = 1
        const val DEFAULT_PORT = 47735

        /** Signs in to a paired computer. */
        fun connect(computer: Computer, listener: Listener): Connection =
            Connection(computer.host, computer.port, computer.fingerprint).apply {
                this.listener = listener
                open { socket ->
                    socket.send(
                        JSONObject().put("type", "hello").put("client", computer.client)
                            .put("token", computer.token).toString()
                    )
                }
            }

        /**
         * Pairs with a computer by the code it shows. [done] gets the computer
         * to remember, or why it failed.
         */
        fun pair(host: String, port: Int, code: String, phoneName: String, done: (Result<Computer>) -> Unit) {
            val connection = Connection(host, port, null)
            val nonce = randomText(16)
            var finished = false
            fun finish(result: Result<Computer>) {
                if (finished) return
                finished = true
                connection.close()
                done(result)
            }
            connection.listener = object : Listener {
                override fun closed(reason: String) = finish(Result.failure(Exception(reason)))
            }
            connection.pairedReply = { reply ->
                val fingerprint = connection.trust.seen ?: ""
                if (reply.optString("proof") != serverProof(code, fingerprint, nonce)) {
                    finish(Result.failure(Exception("That computer couldn't prove it showed this code, so the phone didn't pair. Check you're on the right network, and try again.")))
                } else {
                    finish(Result.success(Computer(
                        reply.optString("server", host), host, port, fingerprint,
                        reply.getString("client"), reply.getString("token"),
                    )))
                }
            }
            connection.open { socket ->
                val fingerprint = connection.trust.seen ?: ""
                socket.send(
                    JSONObject().put("type", "pair").put("name", phoneName).put("nonce", nonce)
                        .put("proof", clientProof(code, fingerprint, nonce)).toString()
                )
            }
        }

        fun normaliseCode(code: String) = code.filter { it.isLetterOrDigit() }.uppercase()

        private fun hmac(code: String, parts: List<String>): String {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(normaliseCode(code).toByteArray(), "HmacSHA256"))
            for (part in parts) {
                mac.update(part.toByteArray())
                mac.update("\n".toByteArray())
            }
            return hex(mac.doFinal())
        }

        fun clientProof(code: String, fingerprint: String, nonce: String) =
            hmac(code, listOf("aae-pair-1 phone", fingerprint, nonce))

        fun serverProof(code: String, fingerprint: String, nonce: String) =
            hmac(code, listOf("aae-pair-1 computer", fingerprint, nonce))

        fun sha256(bytes: ByteArray): String = hex(MessageDigest.getInstance("SHA-256").digest(bytes))

        private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

        private fun randomText(length: Int): String {
            val alphabet = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ"
            val random = SecureRandom()
            return (1..length).map { alphabet[random.nextInt(alphabet.length)] }.joinToString("")
        }
    }
}
