package io.github.aaron_gh.aae.helper

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.content.pm.PermissionInfo
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject

/**
 * The installed apps and their permissions, with the names people see,
 * which adb's package listings don't give.
 */
object AppList {

    /** Every installed app, as a JSON array. */
    fun apps(context: Context): JSONArray {
        val pm = context.packageManager
        val launchable = pm.queryIntentActivities(
            Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0
        ).map { it.activityInfo.packageName }.toSet()
        val out = JSONArray()
        for (info in installed(pm)) {
            val app = info.applicationInfo ?: continue
            out.put(
                JSONObject()
                    .put("package", info.packageName)
                    .put("label", app.loadLabel(pm).toString())
                    .put("version", info.versionName ?: "")
                    .put("system", app.flags and ApplicationInfo.FLAG_SYSTEM != 0)
                    .put("enabled", app.enabled)
                    .put("launchable", info.packageName in launchable)
            )
        }
        return out
    }

    /**
     * The permissions an app asks for that the user grants: runtime
     * permissions, with their names and whether they're granted.
     */
    fun permissions(context: Context, packageName: String): JSONArray {
        val pm = context.packageManager
        val info = packageInfo(pm, packageName, PackageManager.GET_PERMISSIONS)
        val out = JSONArray()
        val names = info.requestedPermissions ?: return out
        val flags = info.requestedPermissionsFlags ?: IntArray(names.size)
        for ((i, name) in names.withIndex()) {
            val permission = try {
                pm.getPermissionInfo(name, 0)
            } catch (e: PackageManager.NameNotFoundException) {
                continue
            }
            val base = if (Build.VERSION.SDK_INT >= 28) permission.protection
            else @Suppress("DEPRECATION") (permission.protectionLevel and PermissionInfo.PROTECTION_MASK_BASE)
            if (base != PermissionInfo.PROTECTION_DANGEROUS) continue
            out.put(
                JSONObject()
                    .put("name", name)
                    .put("label", permission.loadLabel(pm).toString())
                    .put("granted", flags[i] and PackageInfo.REQUESTED_PERMISSION_GRANTED != 0)
            )
        }
        return out
    }

    @Suppress("DEPRECATION")
    private fun installed(pm: PackageManager): List<PackageInfo> =
        if (Build.VERSION.SDK_INT >= 33) pm.getInstalledPackages(PackageManager.PackageInfoFlags.of(0))
        else pm.getInstalledPackages(0)

    @Suppress("DEPRECATION")
    private fun packageInfo(pm: PackageManager, name: String, flags: Int): PackageInfo =
        if (Build.VERSION.SDK_INT >= 33) pm.getPackageInfo(name, PackageManager.PackageInfoFlags.of(flags.toLong()))
        else pm.getPackageInfo(name, flags)
}
