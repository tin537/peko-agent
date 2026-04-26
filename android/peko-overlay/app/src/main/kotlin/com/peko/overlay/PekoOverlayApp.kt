package com.peko.overlay

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Build

/**
 * Peko overlay application — bootstraps the notification channel used by
 * [OverlayService]'s foreground notification and nothing else. The overlay
 * itself lives in the service; this class is deliberately tiny.
 */
class PekoOverlayApp : Application() {

    override fun onCreate() {
        super.onCreate()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NOTIF_CHANNEL_ID,
                getString(R.string.notif_channel_name),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = getString(R.string.notif_channel_desc)
                setShowBadge(false)
            }
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(channel)
        }

        // Phase 5: bring up the audio bridge unconditionally on app
        // start. The service is cheap when idle (FileObserver only) and
        // the agent expects it to be ready as soon as the device is up.
        // BootReceiver also starts it directly on boot.
        try {
            startForegroundService(Intent(this, AudioBridgeService::class.java))
        } catch (_: Throwable) {
            // start-from-app is restricted on some Android versions when
            // the activity context isn't visible; BootReceiver will
            // recover next boot.
        }
    }

    companion object {
        const val NOTIF_CHANNEL_ID = "peko_overlay"
        const val NOTIF_ID = 1
    }
}
