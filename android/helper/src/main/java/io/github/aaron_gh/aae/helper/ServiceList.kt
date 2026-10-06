package io.github.aaron_gh.aae.helper

import android.accessibilityservice.AccessibilityServiceInfo
import android.content.ComponentName
import android.content.Context
import android.view.accessibility.AccessibilityManager
import org.json.JSONArray
import org.json.JSONObject

/**
 * Every accessibility service installed, with the names and descriptions
 * people see in Android's settings, and which are screen readers.
 */
object ServiceList {

    fun services(context: Context): JSONArray {
        val pm = context.packageManager
        val manager = context.getSystemService(AccessibilityManager::class.java)
        val out = JSONArray()
        for (info in manager.installedAccessibilityServiceList) {
            val service = info.resolveInfo.serviceInfo
            val component = ComponentName(service.packageName, service.name)
            // A screen reader can take over touch, to explore the screen by it.
            // TalkBack only asks for it while running, so its manifest's
            // capability is what tells.
            val touch = info.capabilities and AccessibilityServiceInfo.CAPABILITY_CAN_REQUEST_TOUCH_EXPLORATION != 0 ||
                info.flags and AccessibilityServiceInfo.FLAG_REQUEST_TOUCH_EXPLORATION_MODE != 0
            val spoken = info.feedbackType and AccessibilityServiceInfo.FEEDBACK_SPOKEN != 0
            out.put(
                JSONObject()
                    .put("component", component.flattenToString())
                    .put("label", info.resolveInfo.loadLabel(pm).toString())
                    .put("description", info.loadDescription(pm) ?: "")
                    .put("screenReader", touch && spoken)
            )
        }
        return out
    }
}
