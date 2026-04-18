package com.peko.overlay

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
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
    }

    companion object {
        const val NOTIF_CHANNEL_ID = "peko_overlay"
        const val NOTIF_ID = 1
    }
}
