package io.github.aaron_gh.aae.remote

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.ViewConfiguration
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.hypot

/**
 * Touch point mode, inside gesture mode: as gesture mode's keys on the
 * desktop, a point on the device's screen that moves between elements and
 * where gestures happen.
 *
 * - Two-finger swipe right or left: next or previous element.
 * - One-finger swipe: move the point a step that way.
 * - Touch and hold, then drag: the point goes where the finger is, or with
 *   the setting, moves from where it was at half the finger's speed.
 * - Tap or double tap: that gesture at the point. Tap, then touch and hold:
 *   touch and hold at the point until lifted.
 * - Two-finger tap: what's there, and where. Double tap: its properties.
 *   Triple tap: back to the middle.
 */
class TouchPoint(
    private val context: Context,
    private val call: (method: String, params: JSONObject, done: (Any?) -> Unit) -> Unit,
    private val say: (String) -> Unit,
    private val padSize: () -> Pair<Int, Int>,
) {
    private class Target(val label: String, val x: Int, val y: Int, val left: Int, val top: Int, val right: Int, val bottom: Int) {
        fun contains(px: Int, py: Int) = px in left until right && py in top until bottom
        val area get() = (right - left).toLong() * (bottom - top)
        fun same(other: Target?) = other != null && label == other.label && left == other.left &&
            top == other.top && right == other.right && bottom == other.bottom
    }

    private class Screen(val width: Int, val height: Int, val targets: List<Target>) {
        fun under(x: Int, y: Int) = targets.filter { it.contains(x, y) }.minByOrNull { it.area }
    }

    private val main = Handler(Looper.getMainLooper())
    private var point: Pair<Int, Int>? = null
    private var item: Target? = null
    private var label: String? = null
    /** The screen while dragging, read once when the drag starts. */
    private var dragScreen: Screen? = null
    /** Dragging moves the point from where it was, at half the finger's speed,
     *  instead of putting it under the finger. */
    private var relative = false
    /** Where the finger and the point were when the drag began, for `relative`. */
    private var fingerStart = 0f to 0f
    private var pointStart = 0 to 0

    private fun id() = Remote.attached

    private fun screen(then: (Screen) -> Unit) {
        val id = id() ?: return
        call("device.touch_targets", JSONObject().put("id", id)) { result ->
            val json = result as? JSONObject ?: return@call
            val list = json.optJSONArray("targets") ?: JSONArray()
            val targets = (0 until list.length()).map { i ->
                val t = list.getJSONObject(i)
                Target(t.getString("label"), t.getInt("x"), t.getInt("y"), t.getInt("left"), t.getInt("top"), t.getInt("right"), t.getInt("bottom"))
            }
            then(Screen(json.getInt("width"), json.getInt("height"), targets))
        }
    }

    private fun pointOn(screen: Screen) = point ?: (screen.width / 2 to screen.height / 2)

    private fun position(at: Pair<Int, Int>, screen: Screen): String {
        val across = at.first * 100 / maxOf(screen.width, 1)
        val down = at.second * 100 / maxOf(screen.height, 1)
        return "$across percent across, $down percent down, pixel ${at.first}, ${at.second}."
    }

    fun nextItem(next: Boolean) = screen { screen ->
        val targets = screen.targets
        if (targets.isEmpty()) return@screen say("There's nothing on the screen to touch.")
        val at = point
        val index = if (at == null) {
            if (next) 0 else targets.size - 1
        } else {
            val current = targets.indexOfFirst { it.same(item) }.takeIf { it >= 0 }
                ?: screen.under(at.first, at.second)?.let { targets.indexOf(it) }
            when {
                current != null -> current + if (next) 1 else -1
                next -> targets.indexOfFirst { it.top >= at.second }.let { if (it < 0) targets.size else it }
                else -> targets.indexOfLast { it.bottom <= at.second }
            }
        }
        if (index !in targets.indices) return@screen say(if (next) "End of the screen." else "Start of the screen.")
        val target = targets[index]
        point = target.x to target.y
        item = target
        label = target.label
        say(target.label)
    }

    fun step(dx: Int, dy: Int) = screen { screen ->
        val step = maxOf(minOf(screen.width, screen.height) / 10, 1)
        val start = pointOn(screen)
        val x = (start.first + dx * step).coerceIn(0, screen.width - 1)
        val y = (start.second + dy * step).coerceIn(0, screen.height - 1)
        moveTo(x to y, screen, atEdge = x == start.first && y == start.second)
    }

    private fun moveTo(at: Pair<Int, Int>, screen: Screen, atEdge: Boolean = false) {
        point = at
        item = null
        val under = screen.under(at.first, at.second)
        when {
            atEdge -> say("Edge of the screen.")
            under?.label != label -> say(under?.label ?: "Nothing. ${position(at, screen)}")
        }
        label = under?.label
    }

    fun whereIsIt() = screen { screen ->
        val at = pointOn(screen)
        val under = screen.under(at.first, at.second)
        label = under?.label
        say("${under?.label ?: "Nothing."} ${position(at, screen)}")
    }

    fun details() = screen { screen ->
        val at = pointOn(screen)
        val id = id() ?: return@screen
        call("device.details_at", JSONObject().put("id", id).put("x", at.first).put("y", at.second)) { result ->
            say(result as? String ?: "Nothing. ${position(at, screen)}")
        }
    }

    fun centre() {
        point = null
        item = null
        screen { screen ->
            val at = pointOn(screen)
            val under = screen.under(at.first, at.second)
            label = under?.label
            say("Middle of the screen. ${under?.label ?: "Nothing."}")
        }
    }

    private fun gesture(method: String, name: String?) {
        val id = id() ?: return
        val params = JSONObject().put("id", id)
        if (name != null) params.put("gesture", name)
        point?.let { params.put("x", it.first).put("y", it.second) }
        call(method, params) {}
    }

    // MARK: - Touches

    private val config = ViewConfiguration.get(context)
    private val slop = config.scaledTouchSlop.toFloat()
    private val swipeDistance = slop * 4
    private val tapTimeout = ViewConfiguration.getDoubleTapTimeout().toLong()
    private val holdTimeout = ViewConfiguration.getLongPressTimeout().toLong()
    /** Longer than this, a moving finger drags rather than swipes. */
    private val flickTime = 300L

    private enum class State { Pending, Dragging, Holding }

    private var state = State.Pending
    /** The first finger: movement is its movement. */
    private var primary = 0
    private var downAt = 0L
    private var startX = 0f
    private var startY = 0f
    private var lastX = 0f
    private var lastY = 0f
    private var fingers = 0
    private var moved = false
    /** Taps so far, waiting to see if another follows, and with how many fingers. */
    private var taps = 0
    private var tapFingers = 0
    /** This touch followed a one-finger tap: holding it is touch and hold. */
    private var afterTap = false

    private val resolveTaps = Runnable {
        val count = taps
        val with = tapFingers
        taps = 0
        when (with) {
            1 -> gesture("device.gesture", if (count >= 2) "double-tap" else "tap")
            2 -> when (count) {
                1 -> whereIsIt()
                2 -> details()
                else -> centre()
            }
        }
    }

    private val holdCheck = Runnable {
        if (state != State.Pending || moved || fingers != 1) return@Runnable
        if (afterTap) {
            state = State.Holding
            gesture("device.gesture.press", "tap-hold")
        } else {
            startDrag()
        }
    }

    /** Starts a drag: the point follows the finger, from where it is now. */
    private fun startDrag() {
        state = State.Dragging
        dragScreen = null
        relative = relativeDrag(context)
        fingerStart = lastX to lastY
        screen {
            pointStart = pointOn(it)
            dragScreen = it
            // Where the finger is by now.
            if (state == State.Dragging) dragTo(lastX, lastY)
        }
    }

    /** Handles the touch pad's events while touch point mode is on. */
    fun onTouch(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                state = State.Pending
                primary = event.getPointerId(0)
                downAt = event.eventTime
                startX = event.x; startY = event.y
                lastX = event.x; lastY = event.y
                fingers = 1
                moved = false
                afterTap = taps == 1 && tapFingers == 1
                if (afterTap) main.removeCallbacks(resolveTaps)
                main.postDelayed(holdCheck, holdTimeout)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                fingers = maxOf(fingers, event.pointerCount)
                main.removeCallbacks(holdCheck)
            }
            MotionEvent.ACTION_MOVE -> {
                val index = event.findPointerIndex(primary)
                if (index < 0) return
                val x = event.getX(index)
                val y = event.getY(index)
                if (hypot(x - startX, y - startY) > slop) moved = true
                if (state == State.Pending && moved && fingers == 1 && event.eventTime - downAt > flickTime) {
                    main.removeCallbacks(holdCheck)
                    startDrag()
                } else if (state == State.Dragging) {
                    dragTo(x, y)
                }
                lastX = x; lastY = y
            }
            MotionEvent.ACTION_UP -> {
                main.removeCallbacks(holdCheck)
                when (state) {
                    State.Dragging -> dragScreen = null
                    State.Holding -> gesture("device.gesture.release", null)
                    State.Pending -> lifted()
                }
                state = State.Pending
            }
            MotionEvent.ACTION_CANCEL -> {
                main.removeCallbacks(holdCheck)
                if (state == State.Holding) gesture("device.gesture.release", null)
                state = State.Pending
                taps = 0
            }
        }
    }

    private fun lifted() {
        val dx = lastX - startX
        val dy = lastY - startY
        if (moved && hypot(dx, dy) > swipeDistance) {
            taps = 0
            main.removeCallbacks(resolveTaps)
            val horizontal = abs(dx) > abs(dy)
            when {
                fingers >= 2 && horizontal -> nextItem(next = dx > 0)
                fingers == 1 && horizontal -> step(if (dx > 0) 1 else -1, 0)
                fingers == 1 -> step(0, if (dy > 0) 1 else -1)
            }
            return
        }
        if (moved) return
        if (taps > 0 && tapFingers != fingers) taps = 0
        taps++
        tapFingers = fingers
        main.removeCallbacks(resolveTaps)
        main.postDelayed(resolveTaps, tapTimeout)
    }

    /** Puts the point at the finger's place on the pad, scaled to the device's screen. */
    private fun dragTo(x: Float, y: Float) {
        val screen = dragScreen ?: return
        val (padWidth, padHeight) = padSize()
        if (padWidth == 0 || padHeight == 0) return
        val at = if (relative) {
            (pointStart.first + (x - fingerStart.first) * screen.width / padWidth / 2).toInt()
                .coerceIn(0, screen.width - 1) to
                (pointStart.second + (y - fingerStart.second) * screen.height / padHeight / 2).toInt()
                    .coerceIn(0, screen.height - 1)
        } else {
            (x * screen.width / padWidth).toInt().coerceIn(0, screen.width - 1) to
                (y * screen.height / padHeight).toInt().coerceIn(0, screen.height - 1)
        }
        if (at == point) return
        moveTo(at, screen)
    }

    /** Cancels anything waiting, such as when the mode changes. */
    fun reset() {
        main.removeCallbacks(holdCheck)
        main.removeCallbacks(resolveTaps)
        if (state == State.Holding) gesture("device.gesture.release", null)
        state = State.Pending
        taps = 0
    }

    companion object {
        private const val RELATIVE = "touch_point_relative_drag"

        fun relativeDrag(context: Context) =
            context.getSharedPreferences("aae", Context.MODE_PRIVATE).getBoolean(RELATIVE, false)

        fun setRelativeDrag(context: Context, on: Boolean) =
            context.getSharedPreferences("aae", Context.MODE_PRIVATE).edit().putBoolean(RELATIVE, on).apply()
    }
}
