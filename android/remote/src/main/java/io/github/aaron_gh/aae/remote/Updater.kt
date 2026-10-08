package io.github.aaron_gh.aae.remote

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/** A newer AAE Remote. */
data class Update(val version: String, val build: Long, val url: String, val sha256: String, val notes: String)

/**
 * Updates AAE Remote: reads the Android update feed, downloads a newer
 * build, verifies its SHA-256, and passes it to the package installer, which
 * requires user confirmation and the same signing key.
 *
 * Stable releases: appcast-android.json. Development builds, if enabled:
 * the dev prerelease. A development build is offered the next stable release.
 */
object Updater {
    private const val STABLE = "https://raw.githubusercontent.com/aaron-gh/accessible-android-emulator/master/appcast-android.json"
    private const val DEV = "https://github.com/aaron-gh/accessible-android-emulator/releases/download/dev/AAE-Remote-dev.json"
    private const val DAY_MS = 24L * 60 * 60 * 1000

    private val main = Handler(Looper.getMainLooper())
    private val http by lazy {
        OkHttpClient.Builder().connectTimeout(15, TimeUnit.SECONDS).readTimeout(60, TimeUnit.SECONDS).build()
    }

    private fun prefs(context: Context) = context.getSharedPreferences("aae", Context.MODE_PRIVATE)



    /** This app's build number, which the feeds compare against. */
    fun installedBuild(context: Context): Long {
        val info = context.packageManager.getPackageInfo(context.packageName, 0)
        return if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else {
            @Suppress("DEPRECATION")
            info.versionCode.toLong()
        }
    }

    fun installedVersion(context: Context): String =
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""

    /** Whether a day has passed since the last automatic check. Notes this one. */
    fun dueForCheck(context: Context): Boolean {
        val last = prefs(context).getLong("last_check", 0)
        val now = System.currentTimeMillis()
        if (now - last < DAY_MS) return false
        prefs(context).edit().putLong("last_check", now).apply()
        return true
    }

    /** A feed's update, or null when it has none; fails only when the feed can't be reached. */
    private fun read(url: String): Result<Update?> = try {
        http.newCall(Request.Builder().url(url).build()).execute().use { response ->
            if (!response.isSuccessful) return Result.success(null)
            val json = JSONObject(response.body.string())
            Result.success(
                Update(
                    json.getString("version"), json.getLong("build"), json.getString("url"),
                    json.optString("sha256"), json.optString("notes"),
                ).takeIf { it.url.isNotEmpty() }
            )
        }
    } catch (e: Exception) {
        Result.failure(e)
    }

    /** The newest build newer than this one, or null; [done] runs on the main thread. */
    fun check(context: Context, done: (Result<Update?>) -> Unit) {
        val app = context.applicationContext
        val dev = RemoteSettings.offerDevelopmentBuilds
        Thread {
            val results = listOfNotNull(read(STABLE), if (dev) read(DEV) else null)
            val installed = installedBuild(app)
            val newest = results.mapNotNull { it.getOrNull() }.filter { it.build > installed }.maxByOrNull { it.build }
            main.post {
                if (results.all { it.isFailure }) {
                    done(Result.failure(Exception("Couldn't reach AAE's update feed. Check the phone's internet connection.")))
                } else {
                    done(Result.success(newest))
                }
            }
        }.start()
    }

    /** Whether Android lets this app install updates; if not, the settings page that allows it. */
    fun mayInstall(context: Context) = context.packageManager.canRequestPackageInstalls()

    fun allowInstallsIntent(context: Context) =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))

    /**
     * Downloads and installs [update], reporting progress to [said] on the
     * main thread. Android then requires confirmation.
     */
    fun install(context: Context, update: Update, said: (String) -> Unit) {
        val app = context.applicationContext
        Thread {
            try {
                main.post { said("Downloading AAE Remote ${update.version}.") }
                val file = File(app.cacheDir, "update.apk")
                http.newCall(Request.Builder().url(update.url).build()).execute().use { response ->
                    if (!response.isSuccessful) throw Exception("The download failed: ${response.code}.")
                    file.outputStream().use { out -> response.body.byteStream().copyTo(out) }
                }
                if (update.sha256.isNotEmpty()) {
                    val digest = MessageDigest.getInstance("SHA-256")
                    file.inputStream().use { input ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            val n = input.read(buffer)
                            if (n < 0) break
                            digest.update(buffer, 0, n)
                        }
                    }
                    val got = digest.digest().joinToString("") { "%02x".format(it) }
                    if (!got.equals(update.sha256, ignoreCase = true)) {
                        file.delete()
                        throw Exception("The download didn't match AAE's update feed, so it wasn't installed.")
                    }
                }
                val installer = app.packageManager.packageInstaller
                val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
                    setAppPackageName(app.packageName)
                    if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
                }
                val id = installer.createSession(params)
                installer.openSession(id).use { session ->
                    session.openWrite("aae-remote.apk", 0, file.length()).use { out ->
                        file.inputStream().use { it.copyTo(out) }
                        session.fsync(out)
                    }
                    val status = PendingIntent.getBroadcast(
                        app, id, Intent(app, UpdateReceiver::class.java),
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                    )
                    session.commit(status.intentSender)
                }
                file.delete()
                main.post { said("Installing AAE Remote ${update.version}. Android may ask you to confirm.") }
            } catch (e: Exception) {
                main.post { said(e.message ?: "The update couldn't be installed.") }
            }
        }.start()
    }
}

/** Hears from Android's installer, and shows its confirmation when it asks for one. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT)
                }
                confirm?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }
            PackageInstaller.STATUS_SUCCESS -> {}
            else -> {
                val why = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "unknown"
                android.widget.Toast.makeText(context, "AAE Remote couldn't update: $why", android.widget.Toast.LENGTH_LONG).show()
            }
        }
    }
}
