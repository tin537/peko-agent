package com.peko.shim.sms

import android.app.Activity
import android.os.Bundle
import android.util.Log

/**
 * Placeholder activity that exists solely to qualify the package for
 * `android.app.role.SMS`.
 *
 * Role qualification requires an exported activity handling
 * ACTION_SENDTO on the `sms:` and `smsto:` schemes. We don't render any
 * UI and finish immediately — the SMS role is a permissions carrier for
 * peko, not a replacement for the user's preferred messaging client.
 * When another app fires `ACTION_SENDTO`, we hand the intent back to the
 * user's disambiguation chooser by just exiting; other apps further down
 * the priority list pick it up (or the chooser is shown if there's more
 * than one).
 *
 * If you're wondering why this is not a Composable chat screen: peko's
 * real chat UI lives in peko-overlay and on the web :8080 dashboard.
 * Mixing a functional composer into this shim would couple SMS
 * receive/send to chat UX, which we explicitly want to decouple.
 */
class SendToActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.i("PekoSmsShim", "SendToActivity: handoff (data=${intent?.data}) — finishing")
        finish()
    }
}
