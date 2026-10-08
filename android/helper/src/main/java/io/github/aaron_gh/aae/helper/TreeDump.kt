package io.github.aaron_gh.aae.helper

import android.accessibilityservice.AccessibilityService
import android.graphics.Rect
import android.os.Build
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import org.json.JSONArray
import org.json.JSONObject

/**
 * The screen's accessibility tree as JSON, for AAE's accessibility inspector:
 * every window, and in each the nodes a screen reader sees, with what it
 * reads from them.
 */
object TreeDump {
    /** Stop at this many nodes, so a huge screen can't exceed Android's message size. */
    private const val MAX_NODES = 3000

    fun dump(service: AccessibilityService): JSONObject {
        var count = 0
        val windows = JSONArray()
        val list = service.windows.ifEmpty { null }
        if (list != null) {
            for (window in list.sortedByDescending { it.layer }) {
                val root = window.root ?: continue
                val json = describeWindow(window)
                json.put("root", node(root) { count++ < MAX_NODES })
                windows.put(json)
            }
        } else {
            service.rootInActiveWindow?.let { root ->
                windows.put(JSONObject().put("title", "Active window").put("root", node(root) { count++ < MAX_NODES }))
            }
        }
        return JSONObject()
            .put("density", service.resources.displayMetrics.density.toDouble())
            .put("api", Build.VERSION.SDK_INT)
            .put("truncated", count > MAX_NODES)
            .put("windows", windows)
    }

    private fun describeWindow(window: AccessibilityWindowInfo): JSONObject {
        val type = when (window.type) {
            AccessibilityWindowInfo.TYPE_APPLICATION -> "application"
            AccessibilityWindowInfo.TYPE_INPUT_METHOD -> "keyboard"
            AccessibilityWindowInfo.TYPE_SYSTEM -> "system"
            AccessibilityWindowInfo.TYPE_ACCESSIBILITY_OVERLAY -> "accessibility overlay"
            AccessibilityWindowInfo.TYPE_SPLIT_SCREEN_DIVIDER -> "split screen divider"
            else -> "other"
        }
        val json = JSONObject()
            .put("type", type)
            .put("active", window.isActive)
            .put("focused", window.isFocused)
        if (Build.VERSION.SDK_INT >= 24) {
            window.title?.let { json.put("title", it.toString()) }
        }
        return json
    }

    private fun node(info: AccessibilityNodeInfo, budget: () -> Boolean): JSONObject {
        val json = JSONObject()
        if (!budget()) return json.put("truncated", true)
        info.className?.let { json.put("class", it.toString()) }
        info.text?.let { json.put("text", it.toString()) }
        info.contentDescription?.let { json.put("description", it.toString()) }
        info.viewIdResourceName?.let { json.put("id", it) }
        if (Build.VERSION.SDK_INT >= 26) info.hintText?.let { json.put("hint", it.toString()) }
        if (Build.VERSION.SDK_INT >= 30) info.stateDescription?.let { json.put("state", it.toString()) }
        if (Build.VERSION.SDK_INT >= 28) {
            info.paneTitle?.let { json.put("pane", it.toString()) }
            info.tooltipText?.let { json.put("tooltip", it.toString()) }
            if (info.isHeading) json.put("heading", true)
        }
        info.extras?.getCharSequence("AccessibilityNodeInfo.roleDescription")?.let { json.put("role", it.toString()) }
        info.error?.let { json.put("error", it.toString()) }
        info.labeledBy?.let { label ->
            (label.text ?: label.contentDescription)?.let { json.put("labelledBy", it.toString()) }
        }
        val bounds = Rect()
        info.getBoundsInScreen(bounds)
        json.put("bounds", JSONArray().put(bounds.left).put(bounds.top).put(bounds.right).put(bounds.bottom))

        val flags = JSONArray()
        fun flag(name: String, set: Boolean) { if (set) flags.put(name) }
        flag("clickable", info.isClickable)
        flag("long clickable", info.isLongClickable)
        flag("focusable", info.isFocusable)
        flag("focused", info.isFocused)
        flag("accessibility focused", info.isAccessibilityFocused)
        flag("checkable", info.isCheckable)
        flag("checked", info.isChecked)
        flag("selected", info.isSelected)
        flag("disabled", !info.isEnabled)
        flag("editable", info.isEditable)
        flag("password", info.isPassword)
        flag("scrollable", info.isScrollable)
        flag("not visible", !info.isVisibleToUser)
        if (Build.VERSION.SDK_INT >= 24) flag("not important", !info.isImportantForAccessibility)
        if (Build.VERSION.SDK_INT >= 28) flag("screen reader focusable", info.isScreenReaderFocusable)
        json.put("flags", flags)

        val actions = JSONArray()
        for (action in info.actionList) {
            val label = action.label?.toString()
            actions.put(label ?: actionName(action.id))
        }
        json.put("actions", actions)

        // A negative count is an unknown one, as a ListView gives.
        info.collectionInfo?.takeIf { it.rowCount >= 0 }?.let {
            json.put(
                "collection",
                when {
                    it.columnCount > 1 -> "${it.rowCount} rows, ${it.columnCount} columns"
                    it.rowCount == 1 -> "1 item"
                    else -> "${it.rowCount} items"
                },
            )
        }
        info.collectionItemInfo?.let {
            json.put("item", if (it.columnIndex <= 0 && it.columnSpan <= 1) "item ${it.rowIndex + 1}" else "row ${it.rowIndex + 1}, column ${it.columnIndex + 1}")
        }
        info.rangeInfo?.let { json.put("range", "${it.current} of ${it.min} to ${it.max}") }

        val children = JSONArray()
        for (i in 0 until info.childCount) {
            info.getChild(i)?.let { children.put(node(it, budget)) }
        }
        if (children.length() > 0) json.put("children", children)
        return json
    }

    /** Names of actions added in later Android versions, by their fixed ids. */
    private fun actionNameNewer(id: Int): String? = when (id) {
        16908354 -> "move window"
        16908356 -> "show tooltip"
        16908357 -> "hide tooltip"
        16908358 -> "page up"
        16908359 -> "page down"
        16908360 -> "page left"
        16908361 -> "page right"
        16908362 -> "press and hold"
        16908372 -> "IME enter"
        else -> null
    }

    private fun actionName(id: Int): String = when (id) {
        AccessibilityNodeInfo.ACTION_CLICK -> "click"
        AccessibilityNodeInfo.ACTION_LONG_CLICK -> "long click"
        AccessibilityNodeInfo.ACTION_FOCUS -> "focus"
        AccessibilityNodeInfo.ACTION_CLEAR_FOCUS -> "clear focus"
        AccessibilityNodeInfo.ACTION_SELECT -> "select"
        AccessibilityNodeInfo.ACTION_CLEAR_SELECTION -> "clear selection"
        AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> "accessibility focus"
        AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> "clear accessibility focus"
        AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY -> "next at granularity"
        AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY -> "previous at granularity"
        AccessibilityNodeInfo.ACTION_NEXT_HTML_ELEMENT -> "next HTML element"
        AccessibilityNodeInfo.ACTION_PREVIOUS_HTML_ELEMENT -> "previous HTML element"
        AccessibilityNodeInfo.ACTION_SCROLL_FORWARD -> "scroll forward"
        AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> "scroll backward"
        AccessibilityNodeInfo.ACTION_COPY -> "copy"
        AccessibilityNodeInfo.ACTION_PASTE -> "paste"
        AccessibilityNodeInfo.ACTION_CUT -> "cut"
        AccessibilityNodeInfo.ACTION_SET_SELECTION -> "set selection"
        AccessibilityNodeInfo.ACTION_EXPAND -> "expand"
        AccessibilityNodeInfo.ACTION_COLLAPSE -> "collapse"
        AccessibilityNodeInfo.ACTION_DISMISS -> "dismiss"
        AccessibilityNodeInfo.ACTION_SET_TEXT -> "set text"
        android.R.id.accessibilityActionShowOnScreen -> "show on screen"
        android.R.id.accessibilityActionScrollToPosition -> "scroll to position"
        android.R.id.accessibilityActionScrollUp -> "scroll up"
        android.R.id.accessibilityActionScrollLeft -> "scroll left"
        android.R.id.accessibilityActionScrollDown -> "scroll down"
        android.R.id.accessibilityActionScrollRight -> "scroll right"
        android.R.id.accessibilityActionContextClick -> "context click"
        android.R.id.accessibilityActionSetProgress -> "set progress"
        else -> actionNameNewer(id) ?: "action $id"
    }
}
