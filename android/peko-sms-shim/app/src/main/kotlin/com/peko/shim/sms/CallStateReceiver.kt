package com.peko.shim.sms

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.telephony.TelephonyManager
import android.util.Log

/**
 * Companion receiver for outgoing-call number capture. Registered
 * separately so the OFFHOOK handler in [CallStateReceiver] can pull
 * the dialed number out of a side channel rather than trying to
 * mine it from the call log right as the call connects (which is
 * racy — the log row is written after).
 *
 * The Intent extra lives under [Intent.EXTRA_PHONE_NUMBER] and is
 * delivered strictly *before* the call is placed. We stash it on
 * the same companion as the incoming number, and the OFFHOOK
 * handler reads it when deciding what direction + number the
 * recording belongs to.
 */
class OutgoingCallReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_NEW_OUTGOING_CALL) return
        val num = intent.getStringExtra(Intent.EXTRA_PHONE_NUMBER)
        if (!num.isNullOrBlank()) {
            CallStateReceiver.latchOutgoing(num)
            Log.i("PekoCallRcv", "NEW_OUTGOING_CALL to=${num.takeLast(4).padStart(num.length, '*')}")
        }
    }
}

/**
 * Listens for android.intent.action.PHONE_STATE and dispatches the
 * foreground CallRecorderService on call transitions.
 *
 * State model (Android's own `EXTRA_STATE` strings):
 *
 *   IDLE     → no call in progress
 *   RINGING  → incoming call arriving; `EXTRA_INCOMING_NUMBER` extra
 *              carries the caller. We latch the number for the OFFHOOK
 *              transition; the extra is not re-delivered then.
 *   OFFHOOK  → call connected (either user answered incoming, or
 *              outgoing pick-up). Start recording here.
 *
 *   IDLE-after-OFFHOOK → call ended. Stop recording.
 *
 * We deliberately keep last-state + number in a companion object rather
 * than a persisted file: PHONE_STATE is re-delivered to registered
 * receivers on every process start, and calls don't span reboots, so
 * in-memory state is enough and avoids IO on the hot path. The
 * BroadcastReceiver lives in the shim process, not peko-agent, so its
 * lifecycle matches the phone-call lifecycle closely.
 *
 * Outgoing-number capture needs ACTION_NEW_OUTGOING_CALL separately;
 * peko's own dialer path (CallFrameworkTool.dial) goes through
 * `am start ACTION_CALL tel:...` which triggers the same broadcast so
 * we pick up the number from there. For agent-initiated calls the
 * caller is always peko itself, so `direction=outgoing` is enough and
 * we don't need NEW_OUTGOING_CALL wiring.
 */
class CallStateReceiver : BroadcastReceiver() {

    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.action != TelephonyManager.ACTION_PHONE_STATE_CHANGED) return

        val newState = intent.getStringExtra(TelephonyManager.EXTRA_STATE) ?: return
        val number   = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)

        // Duplicate-broadcast guard: Android delivers PHONE_STATE twice
        // on some transitions (once per subscription slot, even on
        // single-SIM devices). Dedupe on state identity.
        val prev = lastState
        if (newState == prev) return
        lastState = newState

        when (newState) {
            TelephonyManager.EXTRA_STATE_RINGING -> {
                // Latch the caller ID for the eventual OFFHOOK
                // (EXTRA_INCOMING_NUMBER is not re-delivered there).
                if (!number.isNullOrBlank()) {
                    latchedNumber = number
                    latchedDirection = "incoming"
                }
                Log.i(TAG, "RINGING from=${redact(number)}")
            }
            TelephonyManager.EXTRA_STATE_OFFHOOK -> {
                // If we didn't see a RINGING first, this is an
                // outgoing call. The NEW_OUTGOING_CALL broadcast
                // fires before PHONE_STATE=OFFHOOK and latches the
                // number via OutgoingCallReceiver → latchOutgoing(),
                // so by the time we're here latchedNumber may be
                // populated with the dialed target.
                if (latchedDirection == null) {
                    latchedDirection = "outgoing"
                }
                val svc = Intent(ctx, CallRecorderService::class.java).apply {
                    action = CallRecorderService.ACTION_START
                    putExtra(CallRecorderService.EXTRA_NUMBER,    latchedNumber ?: "")
                    putExtra(CallRecorderService.EXTRA_DIRECTION, latchedDirection ?: "unknown")
                }
                try {
                    ctx.startForegroundService(svc)
                } catch (e: Throwable) {
                    Log.e(TAG, "startForegroundService failed", e)
                }
                Log.i(TAG, "OFFHOOK dir=$latchedDirection num=${redact(latchedNumber)}")
            }
            TelephonyManager.EXTRA_STATE_IDLE -> {
                // Prev must have been OFFHOOK for there to be
                // anything to stop. RINGING→IDLE = missed call, no
                // recording to tear down.
                if (prev == TelephonyManager.EXTRA_STATE_OFFHOOK) {
                    val svc = Intent(ctx, CallRecorderService::class.java).apply {
                        action = CallRecorderService.ACTION_STOP
                    }
                    try {
                        ctx.startService(svc)
                    } catch (e: Throwable) {
                        Log.e(TAG, "stopService dispatch failed", e)
                    }
                }
                latchedNumber = null
                latchedDirection = null
                Log.i(TAG, "IDLE (prev=$prev)")
            }
        }
    }

    companion object {
        private const val TAG = "PekoCallRcv"

        // In-memory because phone calls never span a reboot.
        @Volatile private var lastState: String? = null
        @Volatile private var latchedNumber: String? = null
        @Volatile private var latchedDirection: String? = null

        private fun redact(n: String?): String {
            if (n.isNullOrBlank()) return "-"
            return if (n.length <= 4) "***" else "***" + n.takeLast(4)
        }

        /**
         * Called from [OutgoingCallReceiver] on
         * ACTION_NEW_OUTGOING_CALL. Runs before the PHONE_STATE =
         * OFFHOOK transition on this thread, so by the time our
         * OFFHOOK branch executes it has the target number in hand.
         */
        fun latchOutgoing(number: String) {
            latchedNumber = number
            latchedDirection = "outgoing"
        }
    }
}
