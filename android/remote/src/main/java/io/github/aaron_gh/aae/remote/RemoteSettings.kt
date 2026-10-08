package io.github.aaron_gh.aae.remote

import android.app.Application
import android.content.Context
import android.content.SharedPreferences
import android.os.Build

/**
 * AAE Remote's settings, in device-protected storage from Android 7. Read
 * once into these fields, and kept up to date by a change listener, so
 * reading one costs nothing. Paired computers aren't settings: their tokens
 * stay in credential-protected storage (see Computers).
 */
object RemoteSettings {
    private const val FILE = "settings"
    private const val DEV_BUILDS = "dev_builds"
    private const val BRIDGE_ENGINE = "bridge_engine"
    private const val BRIDGE_RATE = "bridge_rate"
    private const val RELATIVE_DRAG = "touch_point_relative_drag"

    private lateinit var prefs: SharedPreferences

    /** Offer development builds, not only stable releases. */
    @Volatile var offerDevelopmentBuilds = false
        private set
    /** The speech bridge's engine package, or null for the system default. */
    @Volatile var bridgeEngine: String? = null
        private set
    /** The speech bridge's rate, where 1 is normal, or 0 for the system default. */
    @Volatile var bridgeRate = 0f
        private set
    /** A touch point drag moves the point from where it was, at half the
     *  finger's speed, instead of putting it under the finger. */
    @Volatile var relativeDrag = false
        private set

    // The system holds preference listeners weakly, so this must stay a field.
    private val listener = SharedPreferences.OnSharedPreferenceChangeListener { p, _ -> read(p) }

    fun load(app: Application) {
        val storage = if (Build.VERSION.SDK_INT >= 24) app.createDeviceProtectedStorageContext() else app
        prefs = storage.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        moveOld(app)
        prefs.registerOnSharedPreferenceChangeListener(listener)
        read(prefs)
    }

    private fun read(p: SharedPreferences) {
        offerDevelopmentBuilds = p.getBoolean(DEV_BUILDS, false)
        bridgeEngine = p.getString(BRIDGE_ENGINE, null)
        bridgeRate = p.getFloat(BRIDGE_RATE, 0f)
        relativeDrag = p.getBoolean(RELATIVE_DRAG, false)
    }

    fun setOfferDevelopmentBuilds(on: Boolean) = prefs.edit().putBoolean(DEV_BUILDS, on).apply()

    fun setRelativeDrag(on: Boolean) = prefs.edit().putBoolean(RELATIVE_DRAG, on).apply()

    fun setBridge(engine: String?, rate: Float) =
        prefs.edit().putString(BRIDGE_ENGINE, engine).putFloat(BRIDGE_RATE, rate).apply()

    /** Settings from 0.5.0 and earlier, in credential-protected storage, once. */
    private fun moveOld(app: Context) {
        if (prefs.getBoolean("moved", false)) return
        val old = app.getSharedPreferences("aae", Context.MODE_PRIVATE)
        val bridge = app.getSharedPreferences("bridge_tts", Context.MODE_PRIVATE)
        prefs.edit()
            .putBoolean(DEV_BUILDS, old.getBoolean("dev_builds", false))
            .putBoolean(RELATIVE_DRAG, old.getBoolean(RELATIVE_DRAG, false))
            .putString(BRIDGE_ENGINE, bridge.getString("engine", null))
            .putFloat(BRIDGE_RATE, bridge.getFloat("rate", 0f))
            .putBoolean("moved", true)
            .apply()
        old.edit().remove("dev_builds").remove(RELATIVE_DRAG).apply()
        bridge.edit().clear().apply()
    }
}

/** Loads the settings before anything reads them, and the rules for showing
 *  two screens side by side on wide windows. */
class RemoteApp : Application() {
    override fun onCreate() {
        super.onCreate()
        RemoteSettings.load(this)
        val rules = androidx.window.embedding.RuleController
        rules.getInstance(this).setRules(rules.parseRules(this, R.xml.split_rules))
    }
}
