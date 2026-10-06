package io.github.aaron_gh.aae.helper

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Declares the AAE full keyboard layout. Android finds it through this
 * receiver's meta-data and never actually sends it a broadcast.
 */
class KeyboardLayouts : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = Unit

    companion object {
        /** The layout's descriptor, as Android's input manager names it. */
        const val FULL_KEYBOARD =
            "io.github.aaron_gh.aae.helper/io.github.aaron_gh.aae.helper.KeyboardLayouts/aae_full"
    }
}
