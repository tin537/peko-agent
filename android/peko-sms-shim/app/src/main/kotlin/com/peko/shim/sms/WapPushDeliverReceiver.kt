package com.peko.shim.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Stub MMS handler. Required purely so the package qualifies for
 * `android.app.role.SMS` — RoleController checks for a receiver
 * handling WAP_PUSH_DELIVER with the MMS mime type as part of the
 * SMS-app contract, even though many people never use MMS.
 *
 * We don't actually implement MMS: no MMS provider writes, no download
 * of the M-Retrieve.conf, no notification. Incoming MMS messages are
 * silently dropped while peko holds the SMS role. If you rely on MMS,
 * either (a) revert the SMS role back to your stock messaging app via
 * Settings → Apps → Default apps → SMS, or (b) implement the missing
 * pieces here (not trivial — MMS involves a separate APN, HTTP download
 * of the message body from the carrier's MMSC, PDU parsing). Flagged
 * as future work; see android/peko-sms-shim/README.md.
 */
class WapPushDeliverReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        Log.w(
            "PekoSmsShim",
            "WapPushDeliverReceiver: dropping MMS (mime=${intent.type}, data=${intent.data}) " +
                    "— MMS handling isn't implemented while peko holds the SMS role",
        )
    }
}
