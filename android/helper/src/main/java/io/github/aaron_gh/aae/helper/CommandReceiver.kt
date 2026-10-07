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
 * checks the default speech engine can speak.
 *
 * SPEECH_RELAY (extra: target) sets the real engine the speech relay passes
 * requests to. SPEECH_LOG (extras: since, in milliseconds since 1970; clear)
 * returns what the relay has heard since then, as JSON.
 *
 * DUMP_TREE returns the screen's accessibility tree as JSON in the result
 * data, with result code 1, or 0 if the helper's service isn't running.
 *
 * PLAY_TONE (extras: hz, rate) plays a test tone for a second and a half.
 * The result code is the sample rate it plays at, and the data names the
 * output's native rate.
 *
 * LIST_APPS returns every installed app as JSON: package, label, version,
 * whether it's a system app, enabled, and has an icon to open it with.
 * APP_PERMISSIONS (extra: package) returns the app's runtime permissions as
 * JSON, with their names and whether each is granted. Result code 0 if
 * the app isn't installed.
 *
 * LIST_SERVICES returns every installed accessibility service as JSON:
 * component, label, description, and whether it's a screen reader.
 *
 * And for the check above: The result code is 1 if it can,
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
                // The receiver's own context may not bind to services, which
                // Android 10 and earlier enforce, so use the app's.
                SpeechCheck(context.applicationContext) { result ->
                    pending.setResult(if (result.ok) 1 else 0, "${result.engine}|${result.detail}", null)
                    pending.finish()
                }.start(timeoutMs = 8000)
            }
            ACTION_SPEECH_RELAY -> {
                // The real engine the relay passes requests to.
                val target = intent.getStringExtra("target")
                if (target.isNullOrEmpty() || target == context.packageName) {
                    setResult(0, "no engine given", null)
                } else {
                    SpeechLog.setTarget(context, target)
                    setResult(1, target, null)
                }
            }
            ACTION_SPEECH_LOG -> {
                if (intent.getBooleanExtra("clear", false)) SpeechLog.clear()
                setResult(1, SpeechLog.since(intent.getLongExtra("since", 0)).toString(), null)
            }
            ACTION_MIC_LEVEL -> {
                // Recording takes a moment, so it answers asynchronously.
                val pending = goAsync()
                val ms = intent.getIntExtra("ms", 1000).coerceIn(100, 5000)
                Thread {
                    val peak = MicLevel.peak(ms)
                    pending.setResult(if (peak >= 0) 1 else 0, peak.toString(), null)
                    pending.finish()
                }.start()
            }
            ACTION_SCREEN_CHANGES -> {
                // How many times the screen has changed, and how long it's
                // been still, in milliseconds, for the inspector to follow it.
                if (HelperService.instance == null) {
                    setResult(0, null, null)
                } else {
                    val quiet = android.os.SystemClock.uptimeMillis() - HelperService.lastChange
                    setResult(1, "${HelperService.changes} $quiet", null)
                }
            }
            ACTION_DUMP_TREE -> {
                val service = HelperService.instance
                if (service == null) {
                    setResult(0, null, null)
                } else {
                    setResult(1, TreeDump.dump(service).toString(), null)
                }
            }
            ACTION_PLAY_TONE -> {
                val rate = intent.getIntExtra("rate", 0).takeIf { it > 0 } ?: TestTone.nativeRate()
                TestTone.play(intent.getIntExtra("hz", 1000), rate, 1.5)
                setResult(rate, "native=${TestTone.nativeRate()}", null)
            }
            ACTION_LIST_SERVICES -> {
                setResult(1, ServiceList.services(context).toString(), null)
            }
            ACTION_LIST_APPS -> {
                setResult(1, AppList.apps(context).toString(), null)
            }
            ACTION_APP_PERMISSIONS -> {
                val name = intent.getStringExtra("package").orEmpty()
                try {
                    setResult(1, AppList.permissions(context, name).toString(), null)
                } catch (e: Exception) {
                    setResult(0, e.message, null)
                }
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
        const val ACTION_PLAY_TONE = "io.github.aaron_gh.aae.helper.PLAY_TONE"
        const val ACTION_DUMP_TREE = "io.github.aaron_gh.aae.helper.DUMP_TREE"
        const val ACTION_SCREEN_CHANGES = "io.github.aaron_gh.aae.helper.SCREEN_CHANGES"
        const val ACTION_MIC_LEVEL = "io.github.aaron_gh.aae.helper.MIC_LEVEL"
        const val ACTION_SPEECH_RELAY = "io.github.aaron_gh.aae.helper.SPEECH_RELAY"
        const val ACTION_SPEECH_LOG = "io.github.aaron_gh.aae.helper.SPEECH_LOG"
        const val ACTION_LIST_APPS = "io.github.aaron_gh.aae.helper.LIST_APPS"
        const val ACTION_LIST_SERVICES = "io.github.aaron_gh.aae.helper.LIST_SERVICES"
        const val ACTION_APP_PERMISSIONS = "io.github.aaron_gh.aae.helper.APP_PERMISSIONS"
    }
}
