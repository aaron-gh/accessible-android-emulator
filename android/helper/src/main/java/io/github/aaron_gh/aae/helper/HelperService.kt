package io.github.aaron_gh.aae.helper

import android.accessibilityservice.AccessibilityService
import android.view.accessibility.AccessibilityEvent

/**
 * AAE's accessibility service. While it is on, this app may set the
 * accessibility volume, which Android allows only for accessibility services.
 * It applies the saved volume each time Android connects to it, which is at
 * every boot.
 */
class HelperService : AccessibilityService() {

    override fun onServiceConnected() {
        Volume.applySaved(this)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

}
