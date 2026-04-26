package com.peko.overlay

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.telephony.CellInfoCdma
import android.telephony.CellInfoGsm
import android.telephony.CellInfoLte
import android.telephony.CellInfoNr
import android.telephony.CellInfoWcdma
import android.telephony.CellSignalStrengthNr
import android.telephony.SignalStrength
import android.telephony.TelephonyManager
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.peko.overlay.bridge.RpcDispatcher
import org.json.JSONArray
import org.json.JSONObject

/**
 * Read-only telephony bridge. Three actions:
 *   info    — sim state, carrier, country, phone type, network type
 *   signal  — current registered cell's signal stats
 *   cells   — full neighbour-cell list with per-cell signal
 *
 * No async/streaming yet (telephony deltas would benefit from it; defer
 * until there's a concrete agent need).
 */
class TelephonyBridgeService : Service() {

    private lateinit var tm: TelephonyManager
    private lateinit var dispatcher: RpcDispatcher

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        tm = getSystemService(Context.TELEPHONY_SERVICE) as TelephonyManager
        dispatcher = RpcDispatcher(this, "telephony") { req, _ -> handle(req) }
        dispatcher.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundNow()
        return START_STICKY
    }

    private fun handle(req: JSONObject): RpcDispatcher.Response {
        return when (val action = req.optString("action")) {
            "info" -> RpcDispatcher.Response(doInfo())
            "signal" -> RpcDispatcher.Response(doSignal())
            "cells" -> RpcDispatcher.Response(doCells())
            else -> RpcDispatcher.Response(JSONObject().put("ok", false).put("error", "unknown action '$action'"))
        }
    }

    private fun simStateName(s: Int): String = when (s) {
        TelephonyManager.SIM_STATE_ABSENT -> "ABSENT"
        TelephonyManager.SIM_STATE_PIN_REQUIRED -> "PIN_REQUIRED"
        TelephonyManager.SIM_STATE_PUK_REQUIRED -> "PUK_REQUIRED"
        TelephonyManager.SIM_STATE_NETWORK_LOCKED -> "NETWORK_LOCKED"
        TelephonyManager.SIM_STATE_READY -> "READY"
        TelephonyManager.SIM_STATE_NOT_READY -> "NOT_READY"
        TelephonyManager.SIM_STATE_PERM_DISABLED -> "PERM_DISABLED"
        TelephonyManager.SIM_STATE_CARD_IO_ERROR -> "CARD_IO_ERROR"
        TelephonyManager.SIM_STATE_CARD_RESTRICTED -> "CARD_RESTRICTED"
        else -> "UNKNOWN"
    }

    private fun networkTypeName(t: Int): String = when (t) {
        TelephonyManager.NETWORK_TYPE_LTE -> "LTE"
        TelephonyManager.NETWORK_TYPE_NR -> "5G"
        TelephonyManager.NETWORK_TYPE_HSPA, TelephonyManager.NETWORK_TYPE_HSDPA,
        TelephonyManager.NETWORK_TYPE_HSUPA, TelephonyManager.NETWORK_TYPE_HSPAP -> "HSPA+"
        TelephonyManager.NETWORK_TYPE_UMTS -> "UMTS"
        TelephonyManager.NETWORK_TYPE_EDGE -> "EDGE"
        TelephonyManager.NETWORK_TYPE_GPRS -> "GPRS"
        TelephonyManager.NETWORK_TYPE_GSM -> "GSM"
        TelephonyManager.NETWORK_TYPE_CDMA -> "CDMA"
        TelephonyManager.NETWORK_TYPE_UNKNOWN -> "UNKNOWN"
        else -> "TYPE_$t"
    }

    private fun phoneTypeName(t: Int): String = when (t) {
        TelephonyManager.PHONE_TYPE_GSM -> "GSM"
        TelephonyManager.PHONE_TYPE_CDMA -> "CDMA"
        TelephonyManager.PHONE_TYPE_SIP -> "SIP"
        TelephonyManager.PHONE_TYPE_NONE -> "NONE"
        else -> "UNKNOWN"
    }

    private fun doInfo(): JSONObject {
        val out = JSONObject().put("ok", true)
            .put("sim_state", simStateName(tm.simState))
            .put("phone_type", phoneTypeName(tm.phoneType))
            .put("network_operator_name", tm.networkOperatorName ?: "")
            .put("sim_operator_name", tm.simOperatorName ?: "")
            .put("network_country", tm.networkCountryIso ?: "")
            .put("sim_country", tm.simCountryIso ?: "")
            .put("data_state", when (tm.dataState) {
                TelephonyManager.DATA_DISCONNECTED -> "DISCONNECTED"
                TelephonyManager.DATA_CONNECTING -> "CONNECTING"
                TelephonyManager.DATA_CONNECTED -> "CONNECTED"
                TelephonyManager.DATA_SUSPENDED -> "SUSPENDED"
                else -> "UNKNOWN"
            })
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            out.put("data_network_type", networkTypeName(tm.dataNetworkType))
        }
        // Phone number requires READ_PHONE_NUMBERS / READ_SMS / READ_PHONE_STATE.
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_PHONE_STATE) ==
            PackageManager.PERMISSION_GRANTED) {
            try {
                @Suppress("DEPRECATION")
                val n = tm.line1Number
                if (!n.isNullOrEmpty()) out.put("phone_number", n)
            } catch (_: SecurityException) {}
        }
        return out
    }

    @Suppress("DEPRECATION")
    private fun doSignal(): JSONObject {
        val out = JSONObject().put("ok", true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val ss: SignalStrength? = try { tm.signalStrength } catch (_: SecurityException) { null }
            if (ss != null) {
                out.put("level_0_4", ss.level)
                ss.cellSignalStrengths.firstOrNull()?.let { css ->
                    out.put("dbm", css.dbm)
                    out.put("asu_level", css.asuLevel)
                }
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            out.put("network_type", networkTypeName(tm.dataNetworkType))
        }
        return out
    }

    private fun doCells(): JSONObject {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) !=
            PackageManager.PERMISSION_GRANTED) {
            return JSONObject().put("ok", false).put("error", "ACCESS_FINE_LOCATION required for cell info")
        }
        val cells = JSONArray()
        try {
            tm.allCellInfo?.forEach { info ->
                val c = JSONObject().put("registered", info.isRegistered).put("ts_nanos", info.timeStamp)
                when (info) {
                    is CellInfoLte -> {
                        c.put("type", "LTE")
                            .put("cid", info.cellIdentity.ci)
                            .put("tac", info.cellIdentity.tac)
                            .put("pci", info.cellIdentity.pci)
                            .put("dbm", info.cellSignalStrength.dbm)
                            .put("rsrp", info.cellSignalStrength.rsrp)
                            .put("rsrq", info.cellSignalStrength.rsrq)
                    }
                    is CellInfoNr -> {
                        val ci = info.cellIdentity as android.telephony.CellIdentityNr
                        c.put("type", "5G")
                            .put("nci", ci.nci)
                            .put("tac", ci.tac)
                            .put("pci", ci.pci)
                        (info.cellSignalStrength as? CellSignalStrengthNr)?.let {
                            c.put("dbm", it.dbm)
                            c.put("ss_rsrp", it.ssRsrp)
                            c.put("ss_rsrq", it.ssRsrq)
                        }
                    }
                    is CellInfoGsm -> {
                        c.put("type", "GSM")
                            .put("cid", info.cellIdentity.cid)
                            .put("lac", info.cellIdentity.lac)
                            .put("dbm", info.cellSignalStrength.dbm)
                    }
                    is CellInfoWcdma -> {
                        c.put("type", "WCDMA")
                            .put("cid", info.cellIdentity.cid)
                            .put("lac", info.cellIdentity.lac)
                            .put("dbm", info.cellSignalStrength.dbm)
                    }
                    is CellInfoCdma -> {
                        c.put("type", "CDMA")
                            .put("dbm", info.cellSignalStrength.dbm)
                    }
                    else -> c.put("type", info.javaClass.simpleName)
                }
                cells.put(c)
            }
        } catch (s: SecurityException) {
            return JSONObject().put("ok", false).put("error", "cell info denied: ${s.message}")
        }
        return JSONObject().put("ok", true).put("count", cells.length()).put("cells", cells)
    }

    private fun startForegroundNow() {
        val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            nm.createNotificationChannel(android.app.NotificationChannel(
                NOTIF_CHANNEL_ID, "Peko Telephony Bridge",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) })
        }
        val notif: Notification = NotificationCompat.Builder(this, NOTIF_CHANNEL_ID)
            .setContentTitle("Peko telephony bridge")
            .setContentText("Sim/cell info for the agent")
            .setSmallIcon(android.R.drawable.ic_menu_call)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        dispatcher.stop()
        super.onDestroy()
    }

    companion object {
        const val NOTIF_CHANNEL_ID = "peko_telephony_bridge"
        const val NOTIF_ID = 13
    }
}
