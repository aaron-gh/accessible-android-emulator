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
 */
class CommandReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            ACTION_SET_VOLUME -> {
                val percent = intent.getIntExtra("percent", 100)
                val index = Volume.set(context, percent)
                setResult(index, "Accessibility volume index is $index.", null)
            }
        }
    }

    companion object {
        const val ACTION_SET_VOLUME = "io.github.aaron_gh.aae.helper.SET_VOLUME"
    }
}
