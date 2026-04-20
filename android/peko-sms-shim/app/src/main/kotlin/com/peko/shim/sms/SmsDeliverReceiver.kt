package com.peko.shim.sms

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Telephony
import android.util.Log
import androidx.core.app.NotificationCompat
import java.io.File

/**
 * Writes incoming SMS to `content://sms/inbox` and posts a notification.
 *
 * On Android 4.4+, only the default SMS app receives SMS_DELIVER. If we
 * claim the role and then DROP messages, every incoming text vanishes
 * from the user's phone — no inbox entry, no other app gets a copy.
 * That's a catastrophic behaviour change, so this receiver is
 * load-bearing: the user's stock Messaging app (com.android.messaging
 * on LineageOS) reads from the exact same content provider we write to
 * here, so messages still show up there transparently.
 *
 * We also post a notification because default SMS apps are responsible
 * for user-visible SMS notifications — stock Messaging doesn't post
 * them while it's not the default. The notification click sends the
 * user to stock Messaging via ACTION_VIEW on the sms: URI, so the
 * traditional flow keeps working.
 *
 * Side channel: we append the incoming message to
 * /data/peko/sms_in.log so peko-agent can observe incoming SMS without
 * having to also be the default SMS app itself. Peko's existing reader
 * in src/web/device.rs already queries the inbox DB, so this log is
 * mostly belt-and-braces for event-driven reactions in future.
 */
class SmsDeliverReceiver : BroadcastReceiver() {

    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.action != Telephony.Sms.Intents.SMS_DELIVER_ACTION) return

        val messages = try {
            Telephony.Sms.Intents.getMessagesFromIntent(intent) ?: emptyArray()
        } catch (e: Throwable) {
            Log.e(TAG, "failed to parse SMS_DELIVER intent", e)
            return
        }
        if (messages.isEmpty()) return

        // A multi-part SMS arrives as N messages sharing one address.
        // Concatenate their bodies into a single inbox row so the user
        // sees the full message, not fragments.
        val address = messages.first().originatingAddress ?: "unknown"
        val body = messages.joinToString("") { it.messageBody ?: "" }
        val ts = messages.first().timestampMillis

        // Write to the inbox provider. The stock Messaging app will see
        // it because it reads the same URI.
        try {
            val values = ContentValues().apply {
                put(Telephony.Sms.ADDRESS, address)
                put(Telephony.Sms.BODY,    body)
                put(Telephony.Sms.DATE,      ts)
                put(Telephony.Sms.DATE_SENT, ts)
                put(Telephony.Sms.READ, 0)
                put(Telephony.Sms.SEEN, 0)
                put(Telephony.Sms.TYPE, Telephony.Sms.MESSAGE_TYPE_INBOX)
            }
            val uri = ctx.contentResolver.insert(Telephony.Sms.Inbox.CONTENT_URI, values)
            Log.i(TAG, "incoming SMS from=$address len=${body.length} inserted=$uri")
        } catch (e: Throwable) {
            // If the insert fails, the message is still reachable via
            // the post below — but we should surface the error loudly
            // because it means the user's Messaging app will miss it.
            Log.e(TAG, "inbox insert failed; message may be lost to stock app", e)
        }

        // Originally wrote a side-channel JSON log to /data/peko/sms_in.log,
        // but apps are sandboxed out of /data/peko/ regardless of UNIX
        // perms. peko-agent already reads the telephony provider's inbox
        // table (see src/web/device.rs::get_recent_sms), so this side
        // channel was redundant — removed.

        // Notification. Default SMS app is expected to produce these;
        // stock Messaging suppresses its own when it's not default.
        try {
            postNotification(ctx, address, body)
        } catch (e: Throwable) {
            Log.w(TAG, "failed to post notification", e)
        }
    }

    private fun postNotification(ctx: Context, address: String, body: String) {
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val chan = NotificationChannel(
                NOTIF_CHANNEL_ID,
                "Incoming SMS",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Shows newly-received SMS when peko is the default SMS handler."
                setShowBadge(true)
            }
            nm.createNotificationChannel(chan)
        }

        // Clicking the notification opens the stock Messaging app's
        // conversation view for this sender. Falls back to generic
        // SENDTO if the provider doesn't resolve.
        val viewIntent = Intent(Intent.ACTION_VIEW).apply {
            data = Uri.parse("sms:$address")
            flags = Intent.FLAG_ACTIVITY_NEW_TASK
        }
        val pi = PendingIntent.getActivity(
            ctx,
            address.hashCode(),
            viewIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val preview = if (body.length <= 80) body else body.substring(0, 77) + "…"

        val notif = NotificationCompat.Builder(ctx, NOTIF_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.sym_action_email)
            .setContentTitle(address)
            .setContentText(preview)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()

        // Notification ID = hash of (address, ts truncated). Collision
        // is fine because it just replaces the previous preview card.
        nm.notify(address.hashCode(), notif)
    }

    companion object {
        private const val TAG = "PekoSmsShim"
        private const val NOTIF_CHANNEL_ID = "peko_sms_incoming"
    }
}
