package io.github.aaron_gh.aae.helper

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Commands from AAE, sent as broadcasts from the adb shell:
 *
 *     am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
 *         -a io.github.aaron_gh.aae.helper.SET_VOLUME --ei percent 100
 *
 * The result code is the new volume index, and the result data says it in words.
 *
 *     am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
 *         -a io.github.aaron_gh.aae.helper.KEY_TEST --ez on true
 *
 * turns the keyboard test's key capture on or off. The result code is 1 if
 * the helper's service is running and the change was made, 0 if not.
 *
 *     am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
 *         -a io.github.aaron_gh.aae.helper.CHECK_SPEECH
 *
 * checks the default speech engine can speak. The result code is 1 if it can,
 * 0 if not, and the result data is "<engine package>|<what happened>".
 */
class CommandReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            ACTION_SET_VOLUME -> {
                val percent = intent.getIntExtra("percent", 100)
                val index = Volume.set(context, percent)
                setResult(index, "Accessibility volume index is $index.", null)
            }
            ACTION_CHECK_SPEECH -> {
                // Synthesis takes a moment, so answer asynchronously. Android
                // allows about ten seconds.
                val pending = goAsync()
                SpeechCheck(context) { result ->
                    pending.setResult(if (result.ok) 1 else 0, "${result.engine}|${result.detail}", null)
                    pending.finish()
                }.start(timeoutMs = 8000)
            }
            ACTION_KEY_TEST -> {
                val service = HelperService.instance
                service?.setKeyTest(intent.getBooleanExtra("on", false))
                setResult(if (service != null) 1 else 0, null, null)
            }
        }
    }

    companion object {
        const val ACTION_SET_VOLUME = "io.github.aaron_gh.aae.helper.SET_VOLUME"
        const val ACTION_KEY_TEST = "io.github.aaron_gh.aae.helper.KEY_TEST"
        const val ACTION_CHECK_SPEECH = "io.github.aaron_gh.aae.helper.CHECK_SPEECH"
    }
}
