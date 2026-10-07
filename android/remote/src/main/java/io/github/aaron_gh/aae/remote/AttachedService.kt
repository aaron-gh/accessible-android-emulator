package io.github.aaron_gh.aae.remote

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Keeps the connection, and the device's sound, going while a device is
 * attached, with the screen off or another app in front. Without it, Android
 * freezes the app within seconds of leaving it. It shows the notification
 * Android requires, saying which device is playing.
 */
class AttachedService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val device = intent?.getStringExtra("device") ?: "A device"
        val computer = intent?.getStringExtra("computer") ?: "the computer"
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Device playing", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while a device's audio streams to this phone."
            }
        )
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, ComputersActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle("$device is playing")
            .setContentText("From $computer, through AAE Remote.")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(ID, notification)
        }
        return START_NOT_STICKY
    }

    companion object {
        private const val CHANNEL = "playing"
        private const val ID = 1

        fun start(context: Context, device: String, computer: String) {
            val intent = Intent(context, AttachedService::class.java)
                .putExtra("device", device).putExtra("computer", computer)
            try {
                context.startForegroundService(intent)
            } catch (_: Exception) {
                // Not allowed from the background; the sound still plays while the app is in front.
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, AttachedService::class.java))
        }
    }
}
