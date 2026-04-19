package com.peko.shim.sms

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Binder
import android.os.Build
import android.os.Process
import android.telephony.SmsManager
import android.telephony.SubscriptionManager
import android.util.Log
import java.io.File

/**
 * The only entry point. Reads the intent extras, calls SmsManager, and
 * writes a JSON result file that peko-agent polls.
 *
 * Wire contract (both directions are plain JSON written to flat files
 * under /data/peko/sms_out/ — avoids needing a content provider or
 * bound service in the shim):
 *
 *   PEKO → SHIM      intent action     com.peko.shim.sms.SEND
 *                    extras            id (string), to (E.164 string),
 *                                      body (string), sub_id (int,
 *                                      optional — default SIM if absent)
 *
 *   SHIM → PEKO      file              /data/peko/sms_out/<id>.json
 *                    states            "queued"    request accepted by
 *                                                  the OS, waiting radio
 *                                      "sent"      radio confirmed TX
 *                                      "delivered" recipient ACK'd
 *                                      "error"    any failure, human
 *                                                 readable `error` field
 *
 * Why a file and not a result receiver: peko-agent is a Rust daemon,
 * not an Android app, so it can't register for Intent callbacks. Files
 * are the lowest-friction IPC. Atomic rename (tmp → final) guarantees
 * the poller never reads a half-written JSON.
 */
class SmsCommandReceiver : BroadcastReceiver() {

    override fun onReceive(ctx: Context, intent: Intent) {
        val action = intent.action ?: return
        if (action != ACTION_SEND) {
            Log.w(TAG, "ignored unexpected action $action")
            return
        }

        // ── Caller UID check ──────────────────────────────────────
        // android:permission="SEND_SMS" on the receiver already blocks
        // random apps; this extra check pins the caller to root or
        // shell so a compromised user-installed app that somehow
        // obtained SEND_SMS still can't trigger us. Root = 0,
        // Shell = 2000 on Android.
        val callerUid = Binder.getCallingUid()
        if (callerUid != 0 && callerUid != 2000 && callerUid != Process.myUid()) {
            Log.w(TAG, "rejecting SEND from untrusted uid=$callerUid")
            return
        }

        val id   = intent.getStringExtra(KEY_ID)?.takeIf { it.isNotBlank() }
        val to   = intent.getStringExtra(KEY_TO)?.takeIf { it.isNotBlank() }
        val body = intent.getStringExtra(KEY_BODY)?.takeIf { it.isNotBlank() }

        if (id == null || to == null || body == null) {
            Log.w(TAG, "malformed SEND intent (id/to/body missing)")
            return
        }

        // Optional sub_id for multi-SIM. If unset, let the OS pick the
        // default-outgoing SIM.
        val subId = intent.getIntExtra(KEY_SUB_ID, SubscriptionManager.INVALID_SUBSCRIPTION_ID)

        val manager = try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
                subId != SubscriptionManager.INVALID_SUBSCRIPTION_ID) {
                ctx.getSystemService(SmsManager::class.java)
                    .createForSubscriptionId(subId)
            } else {
                @Suppress("DEPRECATION")
                SmsManager.getDefault()
            }
        } catch (e: Throwable) {
            writeResult(ctx, id, "error", error = "no SmsManager: ${e.message}")
            return
        }

        // Build PendingIntents so the radio's sent/delivered callbacks
        // come back to SmsResultReceiver with our `id` attached. We set
        // FLAG_UPDATE_CURRENT so re-using the same id doesn't stack
        // duplicate pending intents (unlikely but cheap to guard).
        val pendingFlags = PendingIntent.FLAG_ONE_SHOT or
                PendingIntent.FLAG_UPDATE_CURRENT or
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) PendingIntent.FLAG_IMMUTABLE else 0

        fun resultPi(kind: String) = PendingIntent.getBroadcast(
            ctx,
            // requestCode has to be unique per (id, kind) so Android
            // doesn't collapse them onto one extras bundle.
            (id.hashCode() xor kind.hashCode()) and 0x7FFFFFFF,
            Intent(ACTION_RESULT).apply {
                setPackage(ctx.packageName)
                putExtra(KEY_ID, id)
                putExtra(KEY_KIND, kind)
            },
            pendingFlags,
        )

        try {
            manager.sendTextMessage(
                to,
                null,                 // scAddr — always null, let the network decide
                body,
                resultPi("sent"),
                resultPi("delivered"),
            )
            writeResult(ctx, id, "queued", to = to, body_len = body.length)
            Log.i(TAG, "queued SMS id=$id to=$to len=${body.length}")
        } catch (e: Throwable) {
            writeResult(ctx, id, "error", to = to, error = e.message ?: e.javaClass.simpleName)
            Log.e(TAG, "send failed id=$id", e)
        }
    }

    companion object {
        private const val TAG = "PekoSmsShim"

        const val ACTION_SEND   = "com.peko.shim.sms.SEND"
        const val ACTION_STATUS = "com.peko.shim.sms.STATUS"
        const val ACTION_RESULT = "com.peko.shim.sms.RESULT"

        const val KEY_ID     = "id"
        const val KEY_TO     = "to"
        const val KEY_BODY   = "body"
        const val KEY_SUB_ID = "sub_id"
        const val KEY_KIND   = "kind"

        /**
         * Atomically write a JSON status file that peko-agent polls.
         *
         * We go through a tmp file + rename because:
         *   (1) the poller on the Rust side reads the file and parses
         *       JSON — a partially-flushed write would look like a
         *       malformed response
         *   (2) rename() on the same filesystem is atomic per POSIX,
         *       so the poller either sees the old file (still queued)
         *       or the new one (done/error), never a torn read.
         */
        fun writeResult(
            ctx: Context,
            id: String,
            status: String,
            to: String? = null,
            body_len: Int? = null,
            error: String? = null,
        ) {
            val dir = File("/data/peko/sms_out")
            // The dir is created by peko-agent at startup with 0770 and the
            // appropriate group. If it doesn't exist the shim is running
            // without peko, just skip — nothing to do anyway.
            if (!dir.isDirectory) {
                Log.w(TAG, "sms_out dir missing, dropping result id=$id status=$status")
                return
            }

            val json = buildString {
                append('{')
                append("\"id\":\"").append(escape(id)).append("\",")
                append("\"status\":\"").append(escape(status)).append("\",")
                append("\"ts\":").append(System.currentTimeMillis())
                to?.let        { append(",\"to\":\"").append(escape(it)).append("\"") }
                body_len?.let  { append(",\"body_len\":").append(it) }
                error?.let     { append(",\"error\":\"").append(escape(it)).append("\"") }
                append('}')
            }
            val finalFile = File(dir, "$id.json")
            val tmp = File(dir, "$id.json.tmp")
            try {
                tmp.writeText(json)
                if (!tmp.renameTo(finalFile)) {
                    // Rename across mount would fail, but /data/peko stays on
                    // one filesystem. Log and move on — poller will time out.
                    Log.w(TAG, "rename ${tmp} → ${finalFile} failed")
                }
            } catch (e: Throwable) {
                Log.e(TAG, "writeResult io error id=$id", e)
            }
        }

        // Minimal JSON string-escape — covers backslash, quote, control
        // chars and newlines so the Rust poller's serde_json can parse.
        private fun escape(s: String): String {
            val sb = StringBuilder(s.length + 4)
            for (c in s) {
                when (c) {
                    '\\' -> sb.append("\\\\")
                    '"'  -> sb.append("\\\"")
                    '\n' -> sb.append("\\n")
                    '\r' -> sb.append("\\r")
                    '\t' -> sb.append("\\t")
                    else -> if (c.code < 0x20) sb.append("\\u%04x".format(c.code)) else sb.append(c)
                }
            }
            return sb.toString()
        }
    }
}
