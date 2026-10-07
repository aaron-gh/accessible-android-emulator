package io.github.aaron_gh.aae.remote

import android.content.Context
import android.content.Intent
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/**
 * Gesture mode: touches go to the device as they happen, scaled to its
 * screen, with as many pointers as the phone tracks. The AAE gesture mode
 * service passes them through the phone's screen reader. A long press of
 * volume up switches to touch point mode and back (see TouchPoint); a long
 * press of volume down exits; volume up and down change the device's volume.
 *
 * Without the service, or before Android 11, it offers buttons for common
 * gestures instead.
 */
class GestureActivity : ConnectedActivity(), GestureModeService.Mode {
    private lateinit var pad: TouchPad
    /** Touch point mode's answers. An assertive live region, so each one
     *  interrupts the last, as announcements, which queue, don't. */
    private lateinit var spoken: TextView
    private var passthrough = false
    private val touchPoint by lazy {
        TouchPoint(
            this,
            call = { method, params, done -> call(method, params) { done(it) } },
            say = ::say,
            padSize = { pad.width to pad.height },
        )
    }

    private fun say(text: String) {
        // The same text again isn't a change, so wouldn't be spoken.
        spoken.text = if (spoken.text.toString() == text) "$text\u200B" else text
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        val id = Remote.attached ?: return finish()
        val accessibility = getSystemService(AccessibilityManager::class.java)
        passthrough = Build.VERSION.SDK_INT >= 30 &&
            (GestureModeService.isOn(accessibility, packageName) || !GestureModeService.touchExplorationOn(accessibility))
        if (passthrough) {
            pad = TouchPad(this) { touchPoint }
            describePad()
            spoken = TextView(this).apply {
                accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_ASSERTIVE
                setPadding(32, 32, 32, 32)
            }
            setContentView(FrameLayout(this).apply {
                addView(pad)
                addView(spoken, FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                ))
            })
        } else {
            fallback(id)
        }
    }

    /** Buttons for common gestures, and the way to the service. */
    private fun fallback(id: String) {
        ui.heading("Gesture Mode")
        ui.addStatus()
        if (Build.VERSION.SDK_INT >= 30) {
            ui.text("Full-screen gestures need the AAE gesture mode accessibility service.")
            ui.text("If it's a restricted setting: Settings, Apps, AAE Remote, More options, Allow restricted settings. The option appears after one attempt.")
            ui.button("Open Accessibility Settings") {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            }
        } else {
            ui.text("Full-screen gestures need Android 11.")
        }
        val gestures = listOf(
            "Swipe right" to "swipe-right", "Swipe left" to "swipe-left",
            "Swipe up" to "swipe-up", "Swipe down" to "swipe-down",
            "Double tap" to "double-tap", "Double tap and hold" to "double-tap-hold",
            "Swipe up then left" to "swipe-up-left", "Swipe down then right" to "swipe-down-right",
            "Two-finger swipe up" to "two-finger-swipe-up", "Three-finger swipe down" to "three-finger-swipe-down",
        )
        for ((label, gesture) in gestures) {
            ui.button(label) { call("device.gesture", JSONObject().put("id", id).put("gesture", gesture)) }
        }
        ui.show()
    }

    override fun onResume() {
        super.onResume()
        if (passthrough) GestureModeService.keyboard = null
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!passthrough) return
        // A dialog, the notification shade or another app takes the focus,
        // and touch goes back to the phone's screen reader until it returns.
        GestureModeService.gesture = if (hasFocus) this else null
        if (hasFocus) pad.announceForAccessibility("Gesture mode on")
    }

    override fun onPause() {
        if (passthrough && GestureModeService.gesture === this) {
            GestureModeService.gesture = null
            pad.announceForAccessibility("Gesture mode off")
        }
        // Fingers left down would stay down on the device.
        if (passthrough) pad.liftAll()
        super.onPause()
    }

    override fun touchArea(): Rect? {
        if (!passthrough || !pad.isAttachedToWindow) return null
        val at = IntArray(2)
        pad.getLocationOnScreen(at)
        return Rect(at[0], at[1], at[0] + pad.width, at[1] + pad.height)
    }

    override fun volume(up: Boolean) {
        val id = Remote.attached ?: return
        call("device.press", JSONObject().put("id", id).put("key", if (up) "volume-up" else "volume-down"))
    }

    override fun leave() = finish()

    override fun toggle() {
        if (!passthrough) return
        pad.liftAll()
        touchPoint.reset()
        pad.touchPointMode = !pad.touchPointMode
        describePad()
        pad.announceForAccessibility(if (pad.touchPointMode) "Touch point mode" else "Gesture mode")
    }

    private fun describePad() {
        pad.contentDescription = if (pad.touchPointMode) {
            "Touch point mode. Holding volume up switches to gesture mode. Holding volume down exits."
        } else {
            "Gesture mode. Touches go to the device. Holding volume up switches to touch point mode. Holding volume down exits."
        }
    }

    // Without the service, the keys still arrive here.
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || keyCode == KeyEvent.KEYCODE_VOLUME_UP) {
            event.startTracking()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyLongPress(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN) {
            finish()
            return true
        }
        if (keyCode == KeyEvent.KEYCODE_VOLUME_UP) {
            toggle()
            return true
        }
        return super.onKeyLongPress(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN && !event.isCanceled) {
            volume(up = false)
            return true
        }
        if (keyCode == KeyEvent.KEYCODE_VOLUME_UP && !event.isCanceled) {
            volume(up = true)
            return true
        }
        return super.onKeyUp(keyCode, event)
    }

    /** The whole screen, sending each finger to the device, or in touch
     *  point mode, to the touch point. */
    class TouchPad(context: Context, private val touchPoint: () -> TouchPoint) : View(context) {
        private val down = HashSet<Int>()
        var touchPointMode = false

        init {
            isFocusable = true
            layoutParams = android.view.ViewGroup.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.MATCH_PARENT)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            if (touchPointMode) {
                touchPoint().onTouch(event)
                return true
            }
            val connection = Remote.connection ?: return true
            if (width == 0 || height == 0 || Remote.screenWidth == 0) return true
            val points = JSONArray()
            val lifted = when (event.actionMasked) {
                MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> event.actionIndex
                else -> -1
            }
            if (event.actionMasked == MotionEvent.ACTION_CANCEL) {
                liftAll()
                return true
            }
            for (i in 0 until event.pointerCount) {
                val id = event.getPointerId(i)
                val isDown = i != lifted
                if (isDown) down.add(id) else down.remove(id)
                points.put(
                    JSONObject().put("id", id)
                        .put("x", (event.getX(i) * Remote.screenWidth / width).toInt().coerceIn(0, Remote.screenWidth - 1))
                        .put("y", (event.getY(i) * Remote.screenHeight / height).toInt().coerceIn(0, Remote.screenHeight - 1))
                        .put("down", isDown)
                )
            }
            connection.touch(points)
            return true
        }

        fun liftAll() {
            if (down.isEmpty()) return
            val points = JSONArray()
            down.forEach { points.put(JSONObject().put("id", it).put("x", 0).put("y", 0).put("down", false)) }
            down.clear()
            Remote.connection?.touch(points)
        }
    }
}
