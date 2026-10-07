package io.github.aaron_gh.aae.remote

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.graphics.Rect
import android.graphics.Region
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.Display
import android.view.KeyEvent
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityWindowInfo

/**
 * AAE gesture mode: an accessibility service that lets touches through to
 * gesture mode's screen, past the phone's screen reader, so the device's
 * screen reader gets the gestures; and in keyboard mode, takes the keyboard's
 * keys before the phone's screen reader does. It does nothing while neither
 * mode is in front.
 *
 * The touch area is the one NVGT Bridge uses for audio games: Android 11's
 * touch exploration passthrough region. Android keeps one region per display
 * for every accessibility service, and the last one set wins, and some
 * screen readers clear it on each window change, so this sets it again after
 * each one while gesture mode is in front.
 */
class GestureModeService : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())
    private val reassert = Runnable { applyRegion() }
    private var volumeDown = false
    private val leave = Runnable {
        volumeDown = false
        gesture?.leave() ?: keyboard?.leave()
    }
    private var volumeUp = false
    private val toggle = Runnable {
        volumeUp = false
        gesture?.toggle()
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        serviceInfo = serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS or
                AccessibilityServiceInfo.FLAG_REQUEST_FILTER_KEY_EVENTS
        }
        instance = this
        applyRegion()
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        clearRegion()
        instance = null
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        clearRegion()
        instance = null
        super.onDestroy()
    }

    override fun onInterrupt() {}

    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        if (gesture == null) return
        when (event.eventType) {
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED, AccessibilityEvent.TYPE_WINDOWS_CHANGED -> {
                // Once things settle, after anyone else has had their say.
                main.removeCallbacks(reassert)
                main.postDelayed(reassert, 120)
            }
        }
    }

    /** Sets the region to gesture mode's screen, or clears it. */
    fun applyRegion() {
        if (Build.VERSION.SDK_INT < 30) return
        val screen = gesture?.touchArea()
        if (screen == null) return clearRegion()
        val region = Region(screen)
        // The status and navigation bars, the keyboard and other services'
        // overlays stay with the phone's screen reader.
        val bounds = Rect()
        for (window in windows) {
            when (window.type) {
                AccessibilityWindowInfo.TYPE_SYSTEM,
                AccessibilityWindowInfo.TYPE_ACCESSIBILITY_OVERLAY,
                AccessibilityWindowInfo.TYPE_INPUT_METHOD -> {
                    window.getBoundsInScreen(bounds)
                    region.op(bounds, Region.Op.DIFFERENCE)
                }
            }
        }
        try {
            setTouchExplorationPassthroughRegion(Display.DEFAULT_DISPLAY, region)
        } catch (_: Exception) {
        }
    }

    fun clearRegion() {
        main.removeCallbacks(reassert)
        if (Build.VERSION.SDK_INT < 30) return
        try {
            setTouchExplorationPassthroughRegion(Display.DEFAULT_DISPLAY, Region())
        } catch (_: Exception) {
        }
    }

    override fun onKeyEvent(event: KeyEvent): Boolean {
        val mode = gesture ?: keyboard ?: return false
        // A long press of volume down leaves either mode.
        if (event.keyCode == KeyEvent.KEYCODE_VOLUME_DOWN) {
            if (event.action == KeyEvent.ACTION_DOWN && event.repeatCount == 0) {
                volumeDown = true
                main.postDelayed(leave, LONG_PRESS_MS)
            } else if (event.action == KeyEvent.ACTION_UP && volumeDown) {
                volumeDown = false
                main.removeCallbacks(leave)
                mode.volume(up = false)
            }
            return true
        }
        // A long press of volume up switches gesture mode's kind of touch.
        if (event.keyCode == KeyEvent.KEYCODE_VOLUME_UP) {
            if (event.action == KeyEvent.ACTION_DOWN && event.repeatCount == 0) {
                volumeUp = true
                main.postDelayed(toggle, LONG_PRESS_MS)
            } else if (event.action == KeyEvent.ACTION_UP && volumeUp) {
                volumeUp = false
                main.removeCallbacks(toggle)
                mode.volume(up = true)
            }
            return true
        }
        // Keyboard mode takes the keyboard's keys before the phone does.
        return keyboard?.key(event) ?: false
    }

    /** What the service needs from gesture mode and keyboard mode's screens. */
    interface Mode {
        /** Where touches go to the device, in screen pixels, or null for none. */
        fun touchArea(): Rect? = null
        /** A key from the keyboard; true if it went to the device. */
        fun key(event: KeyEvent): Boolean = false
        /** The phone's volume keys change the device's volume. */
        fun volume(up: Boolean)
        fun leave()
        /** Switches between gesture mode and touch point mode. */
        fun toggle() {}
    }

    companion object {
        private const val LONG_PRESS_MS = 800L
        var instance: GestureModeService? = null
            private set
        /** Gesture mode's screen, while it's in front with nothing over it. */
        var gesture: Mode? = null
            set(value) {
                field = value
                instance?.applyRegion()
            }
        /** Keyboard mode's screen, while it's in front. */
        var keyboard: Mode? = null

        /** Whether the user has turned the service on in accessibility settings. */
        fun isOn(manager: AccessibilityManager, packageName: String) =
            manager.getEnabledAccessibilityServiceList(AccessibilityServiceInfo.FEEDBACK_ALL_MASK)
                .any { it.resolveInfo.serviceInfo.packageName == packageName }

        /** Whether a screen reader with explore by touch is on, taking touches. */
        fun touchExplorationOn(manager: AccessibilityManager) = manager.isTouchExplorationEnabled
    }
}
