package io.github.aaron_gh.aae.helper

import android.os.Build
import android.speech.tts.SynthesisCallback
import android.speech.tts.SynthesisRequest
import android.speech.tts.TextToSpeech
import android.speech.tts.TextToSpeechService
import android.speech.tts.UtteranceProgressListener
import android.speech.tts.Voice
import android.util.Log
import java.io.File
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/**
 * AAE's speech relay: a TTS engine that forwards every request to the real
 * engine, recording the text for the speech log. While a speech bridge
 * client is connected, requests go to it instead (see SpeechBridge).
 *
 * The real engine synthesizes to a scratch file. From Android 7, progress
 * callbacks deliver audio as it's produced and it's forwarded immediately.
 * On Android 5 and 6 each utterance is forwarded when complete.
 */
class RelayTtsService : TextToSpeechService() {
    private companion object {
        /** The head start, as a fraction of a second: a tenth. */
        const val HEAD_START_DIVISOR = 10
    }

    private sealed interface Event {
        data class Begin(val rate: Int, val format: Int, val channels: Int) : Event
        class Audio(val bytes: ByteArray) : Event
        object Done : Event
        object Error : Event
    }

    private var engine: TextToSpeech? = null
    private var enginePackage: String? = null
    private var ready = CountDownLatch(1)
    @Volatile private var engineOk = false
    private val queues = ConcurrentHashMap<String, LinkedBlockingQueue<Event>>()
    @Volatile private var stopped = false

    override fun onCreate() {
        connect()
        SpeechBridge.start()
        super.onCreate()
    }

    /** Connects to the real engine: the one AAE chose, or the first other one. */
    @Synchronized
    private fun connect() {
        engine?.shutdown()
        val target = SpeechLog.target(this)?.takeIf { it != packageName } ?: otherEngine()
        enginePackage = target
        val latch = CountDownLatch(1)
        ready = latch
        engineOk = false
        // Never the relay itself: that would loop forever.
        engine = TextToSpeech(applicationContext, { status ->
            engineOk = status == TextToSpeech.SUCCESS
            latch.countDown()
        }, target).also { it.setOnUtteranceProgressListener(listener) }
    }

    /** Another installed engine, preferring Google's, then AAE's eSpeak NG. */
    private fun otherEngine(): String? {
        val engines = packageManager
            .queryIntentServices(android.content.Intent(TextToSpeech.Engine.INTENT_ACTION_TTS_SERVICE), 0)
            .map { it.serviceInfo.packageName }
            .filter { it != packageName }
        return engines.firstOrNull { it == "com.google.android.tts" }
            ?: engines.firstOrNull { it == "io.github.aaron_gh.aae.espeak" }
            ?: engines.firstOrNull()
    }

    override fun onDestroy() {
        engine?.shutdown()
        super.onDestroy()
    }

    private fun engine(): TextToSpeech? {
        ready.await(5, TimeUnit.SECONDS)
        return if (engineOk) engine else null
    }

    private val listener = object : UtteranceProgressListener() {
        override fun onStart(utteranceId: String?) = Unit

        override fun onBeginSynthesis(utteranceId: String?, rate: Int, format: Int, channels: Int) {
            queues[utteranceId]?.put(Event.Begin(rate, format, channels))
        }

        override fun onAudioAvailable(utteranceId: String?, audio: ByteArray?) {
            if (audio != null) queues[utteranceId]?.put(Event.Audio(audio))
        }

        override fun onDone(utteranceId: String?) {
            queues[utteranceId]?.put(Event.Done)
        }

        @Deprecated("Deprecated in Java")
        override fun onError(utteranceId: String?) {
            queues[utteranceId]?.put(Event.Error)
        }

        override fun onError(utteranceId: String?, errorCode: Int) {
            queues[utteranceId]?.put(Event.Error)
        }
    }

    // Languages and voices are the real engine's.

    private fun locale(language: String?, country: String?, variant: String?) =
        Locale(language.orEmpty(), country.orEmpty(), variant.orEmpty())

    override fun onIsLanguageAvailable(lang: String?, country: String?, variant: String?): Int =
        engine()?.isLanguageAvailable(locale(lang, country, variant)) ?: TextToSpeech.LANG_NOT_SUPPORTED

    override fun onLoadLanguage(lang: String?, country: String?, variant: String?): Int =
        engine()?.setLanguage(locale(lang, country, variant)) ?: TextToSpeech.LANG_NOT_SUPPORTED

    override fun onGetLanguage(): Array<String> {
        val locale = engine()?.let { it.voice?.locale ?: it.defaultVoice?.locale } ?: Locale.US
        return arrayOf(
            runCatching { locale.isO3Language }.getOrDefault("eng"),
            runCatching { locale.isO3Country }.getOrDefault(""),
            locale.variant
        )
    }

    override fun onGetVoices(): MutableList<Voice> = engine()?.voices?.toMutableList() ?: mutableListOf()

    override fun onIsValidVoiceName(voiceName: String?): Int =
        if (engine()?.voices?.any { it.name == voiceName } == true) TextToSpeech.SUCCESS else TextToSpeech.ERROR

    override fun onLoadVoice(voiceName: String?): Int {
        val engine = engine() ?: return TextToSpeech.ERROR
        val voice = engine.voices?.firstOrNull { it.name == voiceName } ?: return TextToSpeech.ERROR
        return engine.setVoice(voice)
    }

    override fun onGetDefaultVoiceNameFor(lang: String?, country: String?, variant: String?): String? {
        val engine = engine() ?: return null
        val wanted = locale(lang, country, variant)
        return engine.voices
            ?.filter { it.locale.language == wanted.language && (wanted.country.isEmpty() || it.locale.country == wanted.country) }
            ?.maxByOrNull { it.quality }
            ?.name ?: engine.defaultVoice?.name
    }

    override fun onStop() {
        stopped = true
        SpeechBridge.stop()
        engine?.stop()
        for (queue in queues.values) queue.put(Event.Error)
    }

    override fun onSynthesizeText(request: SynthesisRequest, callback: SynthesisCallback) {
        stopped = false
        // AAE may have chosen another real engine since this one connected.
        val wanted = SpeechLog.target(this)
        if (wanted != null && wanted != packageName && wanted != enginePackage) connect()
        val text = request.charSequenceText?.toString() ?: request.text.orEmpty()
        if (text != SpeechCheck.PHRASE && SpeechLog.recording(this)) SpeechLog.record(text)
        // To the speech bridge client, if connected. The speech check phrase
        // always goes to the real engine.
        if (text != SpeechCheck.PHRASE) {
            val pending = SpeechBridge.send(
                text,
                SpeechBridge.languageTag(request.language, request.country),
                request.speechRate,
                request.pitch,
            )
            if (pending != null) {
                // No sound comes from here, but the framework needs a format.
                callback.start(16000, android.media.AudioFormat.ENCODING_PCM_16BIT, 1)
                pending.await()
                if (!stopped) callback.done()
                return
            }
        }
        val engine = engine() ?: run { callback.error(); return }
        if (request.voiceName != null) {
            engine.voices?.firstOrNull { it.name == request.voiceName }?.let { engine.voice = it }
        } else {
            engine.language = locale(request.language, request.country, request.variant)
        }
        engine.setSpeechRate(request.speechRate / 100f)
        engine.setPitch(request.pitch / 100f)

        val id = "aae-relay-" + System.nanoTime()
        val queue = LinkedBlockingQueue<Event>()
        queues[id] = queue
        val file = File(Storage.cacheDir(this), "$id.wav")
        try {
            if (engine.synthesizeToFile(text, null, file, id) != TextToSpeech.SUCCESS) {
                callback.error()
                return
            }
            if (Build.VERSION.SDK_INT >= 24) relayStream(queue, callback) else relayFile(queue, file, callback)
        } finally {
            queues.remove(id)
            file.delete()
        }
    }

    /**
     * Passes audio on as the real engine produces it (Android 7 and later),
     * after a head start: the first tenth of a second is held back, so a
     * pause in the engine's output, most often just after it starts, doesn't
     * leave Android with nothing to play, which crackles.
     */
    private fun relayStream(queue: LinkedBlockingQueue<Event>, callback: SynthesisCallback) {
        var started = false
        var headStart = 0
        val held = java.io.ByteArrayOutputStream()
        fun release() {
            if (held.size() > 0 && !stopped) write(callback, held.toByteArray())
            held.reset()
            headStart = 0
        }
        while (true) {
            when (val event = queue.poll(15, TimeUnit.SECONDS) ?: Event.Error) {
                is Event.Begin -> {
                    callback.start(event.rate, event.format, event.channels)
                    started = true
                    headStart = event.rate * event.channels * bytesPerSample(event.format) / HEAD_START_DIVISOR
                }
                is Event.Audio -> if (started && !stopped) {
                    if (headStart > 0) {
                        held.write(event.bytes)
                        if (held.size() >= headStart) release()
                    } else {
                        write(callback, event.bytes)
                    }
                }
                Event.Done -> {
                    release()
                    if (started) callback.done() else callback.error()
                    return
                }
                Event.Error -> {
                    if (!stopped) callback.error()
                    return
                }
            }
        }
    }

    private fun bytesPerSample(format: Int) = when (format) {
        android.media.AudioFormat.ENCODING_PCM_8BIT -> 1
        android.media.AudioFormat.ENCODING_PCM_FLOAT -> 4
        else -> 2
    }

    /** Waits for the whole utterance, then passes it on (Android 5 and 6). */
    private fun relayFile(queue: LinkedBlockingQueue<Event>, file: File, callback: SynthesisCallback) {
        while (true) {
            when (queue.poll(30, TimeUnit.SECONDS) ?: Event.Error) {
                Event.Done -> break
                Event.Error -> { if (!stopped) callback.error(); return }
                else -> Unit
            }
        }
        val bytes = file.readBytes()
        if (bytes.size < 44) { callback.error(); return }
        fun le16(at: Int) = (bytes[at].toInt() and 0xff) or ((bytes[at + 1].toInt() and 0xff) shl 8)
        fun le32(at: Int) = le16(at) or (le16(at + 2) shl 16)
        callback.start(le32(24), android.media.AudioFormat.ENCODING_PCM_16BIT, le16(22))
        write(callback, bytes.copyOfRange(44, bytes.size))
        callback.done()
    }

    private fun write(callback: SynthesisCallback, bytes: ByteArray) {
        val max = callback.maxBufferSize
        var offset = 0
        while (offset < bytes.size && !stopped) {
            val length = minOf(max, bytes.size - offset)
            if (callback.audioAvailable(bytes, offset, length) != TextToSpeech.SUCCESS) {
                Log.w(Volume.TAG, "the speech relay couldn't pass audio on")
                return
            }
            offset += length
        }
    }
}
