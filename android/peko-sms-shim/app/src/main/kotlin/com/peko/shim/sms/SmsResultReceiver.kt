package com.peko.shim.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.telephony.SmsManager
import android.util.Log

/**
 * Fired by the PendingIntents SmsCommandReceiver wired up on send.
 *
 * The result code the radio returns is the source of truth — we map
 * it to a status string and re-write the JSON file so peko-agent's
 * poller sees the final outcome.
 *
 *   SmsManager.RESULT_ERROR_* codes are documented at
 *   https://developer.android.com/reference/android/telephony/SmsManager
 *
 * This receiver is NOT exported (see AndroidManifest) — only our own
 * PendingIntent can trigger it, so no uid check is needed.
 */
class SmsResultReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.action != SmsCommandReceiver.ACTION_RESULT) return

        val id   = intent.getStringExtra(SmsCommandReceiver.KEY_ID)   ?: return
        val kind = intent.getStringExtra(SmsCommandReceiver.KEY_KIND) ?: return

        val code = resultCode
        val status: String
        val error: String?
        when (kind) {
            "sent" -> {
                status = if (code == android.app.Activity.RESULT_OK) "sent" else "error"
                error = if (code != android.app.Activity.RESULT_OK) smsSendErrorName(code) else null
            }
            "delivered" -> {
                // The delivery PDU is in intent.getByteArrayExtra("pdu") for
                // fancier UIs, but the mere fact we got a callback with
                // RESULT_OK means the recipient's network acknowledged.
                status = if (code == android.app.Activity.RESULT_OK) "delivered" else "error"
                error = if (code != android.app.Activity.RESULT_OK) "delivery failed (code=$code)" else null
            }
            else -> {
                Log.w(TAG, "unexpected result kind=$kind for id=$id")
                return
            }
        }

        SmsCommandReceiver.writeResult(ctx, id, status, error = error)
        Log.i(TAG, "result id=$id kind=$kind status=$status err=$error")
    }

    private fun smsSendErrorName(code: Int): String = when (code) {
        SmsManager.RESULT_ERROR_GENERIC_FAILURE -> "generic failure"
        SmsManager.RESULT_ERROR_NO_SERVICE      -> "no service / no signal"
        SmsManager.RESULT_ERROR_NULL_PDU        -> "null PDU"
        SmsManager.RESULT_ERROR_RADIO_OFF       -> "radio off (airplane mode?)"
        SmsManager.RESULT_ERROR_LIMIT_EXCEEDED  -> "SMS send rate exceeded (carrier)"
        SmsManager.RESULT_ERROR_FDN_CHECK_FAILURE -> "FDN check failure"
        else -> "sms send failed (code=$code)"
    }

    companion object {
        private const val TAG = "PekoSmsShim"
    }
}
