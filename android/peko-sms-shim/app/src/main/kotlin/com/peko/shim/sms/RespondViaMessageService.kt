package com.peko.shim.sms

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.util.Log

/**
 * Stub for `android.intent.action.RESPOND_VIA_MESSAGE`.
 *
 * Android fires this action when the user taps the "reply with text"
 * shortcut on an incoming-call screen. RoleController requires any SMS
 * role holder to expose a service handling it, even though the action
 * is rarely used in practice.
 *
 * Real implementation would parse the recipient + body from the intent
 * extras and fire an SmsManager.sendTextMessage — effectively a
 * self-dispatch through the same path SmsCommandReceiver uses. Because
 * the reply is expected to be a human typing on the call screen, and
 * peko isn't in the loop there, we intentionally do nothing. If the
 * user decides peko should draft these replies, that's a separate
 * feature gated behind an explicit config flag.
 */
class RespondViaMessageService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i("PekoSmsShim", "RespondViaMessageService: stub — ignoring ${intent?.data}")
        stopSelf(startId)
        return START_NOT_STICKY
    }
}
