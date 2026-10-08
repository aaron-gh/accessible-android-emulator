package io.github.aaron_gh.aae.remote

import android.os.Bundle
import android.view.KeyCharacterMap
import android.view.KeyEvent
import java.util.Locale
import android.widget.TextView
import org.json.JSONObject

/**
 * Keyboard mode: every key of a keyboard attached to the phone goes to the
 * device, by its Linux key code, which Android gives as the key's scan code.
 * Keys typing text, without Control, left Alt or Meta, also carry the
 * character the phone's layout typed, and the device types that character
 * on its own layout, chosen for the keyboard's language. Dead keys are
 * combined with the next key here, as a text field would.
 * Control-Shift-Escape or a long press of volume down exits; plain Escape
 * goes to the device.
 *
 * With AAE gesture mode's service on, keys go to the device before the
 * phone's screen reader can take them for its own shortcuts. Without it, the
 * screen reader and Android keep theirs.
 */
class KeyboardActivity : ConnectedActivity(), GestureModeService.Mode {
    private lateinit var surface: TextView
    private val held = HashSet<Int>()
    /** A dead key's accent, waiting for the next key. */
    private var accent = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (isFinishing) return
        if (Remote.attached == null) return finish()
        ui.heading("Keyboard Mode")
        surface = ui.text(
            "Keys go to the device. Control-Shift-Escape, or holding volume down, exits."
        )
        surface.isFocusable = true
        surface.isFocusableInTouchMode = true
        ui.show()
        surface.requestFocus()
    }

    override fun onResume() {
        super.onResume()
        GestureModeService.keyboard = this
        Remote.connection?.keyboard(keyboardLanguage())
        surface.announceForAccessibility("Keyboard mode on")
    }

    override fun onPause() {
        if (GestureModeService.keyboard === this) GestureModeService.keyboard = null
        // Keys left down would stay down on the device.
        val connection = Remote.connection
        held.forEach { connection?.key(it, false, "") }
        held.clear()
        accent = 0
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
        // Releases always go as text keys: the server releases whatever the
        // press sent.
        Remote.connection?.key(code, down, if (down) typed(event) else "")
        return true
    }

    /** The character a key types, "" for a dead key, or null for a shortcut
     *  or a key typing nothing, which go by position. */
    private fun typed(event: KeyEvent): String? {
        val shortcut = KeyEvent.META_CTRL_ON or KeyEvent.META_ALT_LEFT_ON or KeyEvent.META_META_ON
        if (event.metaState and shortcut != 0) return null
        val unicode = event.getUnicodeChar(event.metaState)
        if (unicode == 0) return null
        if (unicode and KeyCharacterMap.COMBINING_ACCENT != 0) {
            accent = unicode and KeyCharacterMap.COMBINING_ACCENT_MASK
            return ""
        }
        val pending = accent
        accent = 0
        if (pending != 0) {
            val combined = KeyCharacterMap.getDeadChar(pending, unicode)
            if (combined != 0) return String(Character.toChars(combined))
        }
        return String(Character.toChars(unicode))
    }

    /** The phone's language, for the device's layout: characters are sent as
     *  typed, so its layout only needs to have them. */
    private fun keyboardLanguage(): String = Locale.getDefault().toLanguageTag()

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
