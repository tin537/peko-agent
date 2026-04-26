package com.peko.overlay

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.util.Log

/**
 * Starts the overlay automatically on every boot.
 *
 * Fires on ACTION_BOOT_COMPLETED (after user credential unlock on an
 * FBE device; Android 13 / LineageOS 20 always uses FBE on OnePlus
 * 6T). Confirms that SYSTEM_ALERT_WINDOW is actually granted before
 * launching the foreground service — without that perm the service
 * would start only to have WindowManager throw when it tries to add
 * the overlay view, leaving a silent zombie notification.
 *
 * Android 13 carves out a narrow exemption: FGS *can* be started
 * from a BOOT_COMPLETED receiver even though the normal "no FGS
 * from background" rule would block it. We rely on that here.
 *
 * When peko-overlay ships as a Magisk priv-app SYSTEM_ALERT_WINDOW
 * is auto-granted by service.sh via `appops`, so the happy path on
 * a fresh install is: device boots → this receiver fires →
 * canDrawOverlays true → OverlayService starts → peko cat appears
 * with zero user taps required. That's the "agent as OS" contract
 * — the overlay should be ambient, not something you have to
 * re-launch.
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(ctx: Context, intent: Intent) {
        val action = intent.action ?: return
        val expected = action == Intent.ACTION_BOOT_COMPLETED ||
                action == Intent.ACTION_LOCKED_BOOT_COMPLETED ||
                action == "android.intent.action.QUICKBOOT_POWERON"
        if (!expected) return

        if (!Settings.canDrawOverlays(ctx)) {
            // Fresh install case: service.sh races us to grant the
            // appop. If it hasn't yet, silently bail — service.sh
            // also invokes `am start` on MainActivity as a backup
            // once the appop is set, which will reach this code path
            // effectively.
            Log.i(TAG, "overlay perm not yet granted; skipping autostart")
            return
        }

        val services = listOf(
            OverlayService::class.java,
            AudioBridgeService::class.java,
            LocationBridgeService::class.java,
            CameraBridgeService::class.java,
            TelephonyBridgeService::class.java,
        )
        for (cls in services) {
            val svc = Intent(ctx, cls)
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    ctx.startForegroundService(svc)
                } else {
                    ctx.startService(svc)
                }
            } catch (e: Throwable) {
                Log.e(TAG, "autostart failed for ${cls.simpleName}", e)
            }
        }
        Log.i(TAG, "overlay + bridge services autostart kicked via $action")
    }

    companion object { private const val TAG = "PekoOverlayBoot" }
}
