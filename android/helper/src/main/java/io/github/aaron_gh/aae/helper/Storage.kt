package io.github.aaron_gh.aae.helper

import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import java.io.File

/**
 * The helper's storage. From Android 7 it uses device-protected storage, so
 * the helper works before the user unlocks a device with a screen lock after
 * a cold boot. Earlier versions have only one kind of storage.
 */
object Storage {
    private const val PREFS = "aae"
    @Volatile private var moved = false

    fun prefs(context: Context): SharedPreferences {
        if (Build.VERSION.SDK_INT < 24) return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val protected = context.createDeviceProtectedStorageContext()
        // Settings saved by helper 0.27 and earlier are in credential-protected
        // storage. Moving them needs the user unlocked; until then, defaults
        // apply. Settings already saved here are newer, and stay.
        if (!moved && isUnlocked(context)) {
            if (!File(protected.dataDir, "shared_prefs/$PREFS.xml").exists()) {
                protected.moveSharedPreferencesFrom(context, PREFS)
            }
            moved = true
        }
        return protected.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    /** A cache folder usable before the user unlocks the device. */
    fun cacheDir(context: Context): File =
        if (Build.VERSION.SDK_INT >= 24) context.createDeviceProtectedStorageContext().cacheDir
        else context.cacheDir

    private fun isUnlocked(context: Context): Boolean =
        (context.getSystemService(Context.USER_SERVICE) as android.os.UserManager).isUserUnlocked
}
