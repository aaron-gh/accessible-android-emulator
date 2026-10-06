package io.github.aaron_gh.aae.helper

import android.accessibilityservice.AccessibilityService
import android.util.Log
import android.view.KeyEvent
import android.view.accessibility.AccessibilityEvent

/**
 * AAE's accessibility service. While it is on, this app may set the
 * accessibility volume, which Android allows only for accessibility services.
 * It applies the saved volume each time Android connects to it, which is at
 * every boot.
 *
 * It also runs AAE's keyboard test. While the test runs it logs each key and
 * swallows it, so the test's keys reach no app. At all other times it passes
 * every key straight on without looking at it.
 */
class HelperService : AccessibilityService() {

    override fun onServiceConnected() {
        instance = this
        Volume.applySaved(this)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    /**
     * Turns the keyboard test's key capture on or off. Key filtering itself is
     * declared in the service's configuration and never changed at run time:
     * Android 11 ignores it being turned back on.
     */
    fun setKeyTest(on: Boolean) {
        keyTest = on
    }

    override fun onKeyEvent(event: KeyEvent): Boolean {
        if (!keyTest) return false
        Log.i(
            KEY_TAG,
            "${KeyEvent.keyCodeToString(event.keyCode)} " +
                (if (event.action == KeyEvent.ACTION_DOWN) "down" else "up") +
                " meta=0x${Integer.toHexString(event.metaState)}"
        )
        return true
    }

    private var keyTest = false

    companion object {
        /** The log tag AAE reads the keyboard test's results from. */
        const val KEY_TAG = "AaeKeys"

        @Volatile
        var instance: HelperService? = null
            private set
    }
}
