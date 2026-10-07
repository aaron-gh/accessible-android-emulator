package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.view.KeyEvent
import android.widget.TextView
import org.json.JSONObject

/**
 * Keyboard mode: every key of a keyboard attached to the phone goes to the
 * device, by its Linux key code, which Android gives as the key's scan code.
 * Control, Shift and Escape together come back, as does a long press of
 * volume down; plain Escape goes to the device.
 *
 * With AAE gesture mode's service on, keys go to the device before the
 * phone's screen reader can take them for its own shortcuts. Without it, the
 * screen reader and Android keep theirs.
 */
class KeyboardActivity : ConnectedActivity(), GestureModeService.Mode {
    private lateinit var surface: TextView
    private val held = HashSet<Int>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        if (Remote.attached == null) return finish()
        ui.heading("Keyboard Mode")
        surface = ui.text(
            "Keys go to the device. Press Control, Shift and Escape together, or hold volume down, to come back."
        )
        surface.isFocusable = true
        surface.isFocusableInTouchMode = true
        ui.show()
        surface.requestFocus()
    }

    override fun onResume() {
        super.onResume()
        GestureModeService.keyboard = this
        surface.announceForAccessibility("Keyboard mode on")
    }

    override fun onPause() {
        if (GestureModeService.keyboard === this) GestureModeService.keyboard = null
        // Keys left down would stay down on the device.
        val connection = Remote.connection
        held.forEach { connection?.key(it, false) }
        held.clear()
        super.onPause()
    }

    override fun key(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_ESCAPE && event.isCtrlPressed && event.isShiftPressed) {
            if (event.action == KeyEvent.ACTION_UP) leave()
            return true
        }
        val code = event.scanCode
        if (code <= 0) return false
        val down = event.action == KeyEvent.ACTION_DOWN
        if (down) held.add(code) else held.remove(code)
        Remote.connection?.key(code, down)
        return true
    }

    override fun volume(up: Boolean) {
        val id = Remote.attached ?: return
        call("device.press", JSONObject().put("id", id).put("key", if (up) "volume-up" else "volume-down"))
    }

    override fun leave() {
        surface.announceForAccessibility("Keyboard mode off")
        finish()
    }

    // Without the service, keys come here instead, less those the system keeps.
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || event.keyCode == KeyEvent.KEYCODE_VOLUME_UP) {
            return super.dispatchKeyEvent(event)
        }
        return key(event) || super.dispatchKeyEvent(event)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN) {
            event.startTracking()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyLongPress(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN) {
            leave()
            return true
        }
        return super.onKeyLongPress(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        if ((keyCode == KeyEvent.KEYCODE_VOLUME_DOWN && !event.isCanceled) || keyCode == KeyEvent.KEYCODE_VOLUME_UP) {
            volume(up = keyCode == KeyEvent.KEYCODE_VOLUME_UP)
            return true
        }
        return super.onKeyUp(keyCode, event)
    }
}
