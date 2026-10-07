package io.github.aaron_gh.aae.helper

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import java.io.File

/**
 * Checks that the device's default speech engine can actually speak, by
 * having it synthesize a short phrase to a file. Nothing is played aloud.
 *
 * A screen reader that can't speak is useless, and speech can fail quietly:
 * Google's engine sometimes downloads voices that then fail to load, so every
 * request fails while the screen reader's earcons still play.
 */
class SpeechCheck(private val context: Context, private val done: (Result) -> Unit) {

    data class Result(val ok: Boolean, val engine: String, val detail: String)

    companion object {
        /** What the check says. The speech log leaves it out. */
        const val PHRASE = "Accessible Android Emulator speech check."
    }

    private val handler = Handler(Looper.getMainLooper())
    private var tts: TextToSpeech? = null
    private var finished = false
    private val engine: String =
        Settings.Secure.getString(context.contentResolver, "tts_default_synth") ?: ""

    fun start(timeoutMs: Long) {
        handler.postDelayed({ finish(false, "the speech engine did not answer in time") }, timeoutMs)
        tts = TextToSpeech(context) { status ->
            if (status != TextToSpeech.SUCCESS) {
                finish(false, "the speech engine did not start")
                return@TextToSpeech
            }
            synthesize()
        }
    }

    private fun synthesize() {
        val engine = tts ?: return
        val file = File(Storage.cacheDir(context), "speech-check.wav")
        engine.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) = Unit

            override fun onDone(utteranceId: String?) {
                val bytes = if (file.exists()) file.length() else 0
                // A WAV header alone is 44 bytes; real speech is far more.
                if (bytes > 1000) finish(true, "$bytes bytes of speech") else finish(false, "the speech was empty")
            }

            @Deprecated("Deprecated in Java")
            override fun onError(utteranceId: String?) = finish(false, "the speech engine reported an error")

            override fun onError(utteranceId: String?, errorCode: Int) =
                finish(false, "the speech engine reported error $errorCode")
        })
        val result = engine.synthesizeToFile(PHRASE, null, file, "aae-check")
        if (result != TextToSpeech.SUCCESS) {
            finish(false, "the speech engine refused the request")
        }
    }

    private fun finish(ok: Boolean, detail: String) {
        handler.post {
            if (finished) return@post
            finished = true
            handler.removeCallbacksAndMessages(null)
            tts?.shutdown()
            val name = engine.ifEmpty { tts?.defaultEngine ?: "none" }
            done(Result(ok, name, detail))
        }
    }
}
