package io.github.aaron_gh.aae.remote

import android.content.Context
import android.media.AudioAttributes
import android.os.Bundle
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import java.util.Locale

/**
 * Speech bridge output on the phone, using the engine and rate from Bridge
 * Text-to-Speech Settings, or the system defaults. Reports each utterance's
 * completion or stop to the device.
 */
class PhoneVoice(context: Context, private val done: (Long) -> Unit) {
    private var ready = false
    private var failed = false
    /** What arrived before the engine was ready. */
    private var waiting: Triple<Long, String, String>? = null
    private val tts: TextToSpeech

    init {
        val rate = RemoteSettings.bridgeRate
        tts = TextToSpeech(context.applicationContext, { status ->
            synchronized(this) {
                ready = status == TextToSpeech.SUCCESS
                failed = !ready
                tts.setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_ASSISTANCE_ACCESSIBILITY)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build()
                )
                tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                    override fun onStart(utteranceId: String?) = Unit
                    override fun onDone(utteranceId: String?) = finished(utteranceId)
                    override fun onStop(utteranceId: String?, interrupted: Boolean) = finished(utteranceId)
                    @Deprecated("Deprecated in Java")
                    override fun onError(utteranceId: String?) = finished(utteranceId)
                    override fun onError(utteranceId: String?, errorCode: Int) = finished(utteranceId)
                })
                if (rate > 0f) tts.setSpeechRate(rate)
                val next = waiting
                waiting = null
                if (next != null) speak(next.first, next.second, next.third)
            }
        }, RemoteSettings.bridgeEngine)
    }

    private fun finished(utteranceId: String?) {
        utteranceId?.toLongOrNull()?.let(done)
    }

    @Synchronized
    fun speak(id: Long, text: String, language: String) {
        if (failed || text.isBlank()) return done(id)
        if (!ready) {
            waiting?.let { done(it.first) }
            waiting = Triple(id, text, language)
            return
        }
        // Default voice for the system language; otherwise one for the utterance's language.
        val wanted = Locale.forLanguageTag(language)
        if (language.isNotEmpty() && wanted.language != Locale.getDefault().language &&
            tts.isLanguageAvailable(wanted) >= TextToSpeech.LANG_AVAILABLE
        ) {
            tts.language = wanted
        } else {
            tts.defaultVoice?.let { if (tts.voice != it) tts.voice = it }
        }
        if (tts.speak(text, TextToSpeech.QUEUE_FLUSH, Bundle(), id.toString()) != TextToSpeech.SUCCESS) done(id)
    }

    @Synchronized
    fun stop() {
        waiting?.let { done(it.first) }
        waiting = null
        if (ready) tts.stop()
    }

    fun release() {
        tts.stop()
        tts.shutdown()
    }
}
