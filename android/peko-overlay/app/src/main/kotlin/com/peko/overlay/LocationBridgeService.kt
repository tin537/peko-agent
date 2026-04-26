package com.peko.overlay

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.peko.overlay.bridge.EventStore
import com.peko.overlay.bridge.RpcDispatcher
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference

/**
 * GPS / fused-location bridge. Three actions:
 *
 *   fix { timeout_ms?:30000, provider?:"gps"|"network"|"fused" }
 *     One-shot: tries last-known first, then registers a single-update
 *     listener; returns the first fresh sample or times out.
 *
 *   start_stream { stream_id?, interval_ms?:5000, min_distance_m?:0,
 *                  provider?:"gps" }
 *     Begins continuous updates; each callback is appended to the shared
 *     EventStore as type="location". Returns immediately.
 *
 *   stop_stream { stream_id }
 *     Unregisters the listener for that id.
 *
 * The agent reads samples via the events poller — no per-fix file roundtrip.
 */
class LocationBridgeService : Service() {

    private lateinit var lm: LocationManager
    private lateinit var dispatcher: RpcDispatcher
    private val activeStreams = ConcurrentHashMap<String, LocationListener>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        lm = getSystemService(Context.LOCATION_SERVICE) as LocationManager
        dispatcher = RpcDispatcher(this, "location") { req, ctx -> handle(req, ctx) }
        dispatcher.start()
        Log.i(TAG, "location bridge ready")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundNow()
        return START_STICKY
    }

    private fun handle(req: JSONObject, ctx: RpcDispatcher.RpcContext): RpcDispatcher.Response {
        val action = req.optString("action")
        return when (action) {
            "fix" -> doFix(req)
            "start_stream" -> doStartStream(req)
            "stop_stream" -> doStopStream(req)
            else -> RpcDispatcher.Response(JSONObject().put("ok", false).put("error", "unknown action '$action'"))
        }
    }

    private fun pickProvider(name: String): String {
        return when (name) {
            "network" -> LocationManager.NETWORK_PROVIDER
            "fused" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) LocationManager.FUSED_PROVIDER else LocationManager.GPS_PROVIDER
            else -> LocationManager.GPS_PROVIDER
        }
    }

    private fun hasFineLocPerm(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    private fun doFix(req: JSONObject): RpcDispatcher.Response {
        if (!hasFineLocPerm()) return errResp("ACCESS_FINE_LOCATION not granted")
        val provider = pickProvider(req.optString("provider", "gps"))
        if (!lm.isProviderEnabled(provider)) {
            return errResp("provider '$provider' is disabled (toggle Location in Settings)")
        }
        val timeoutMs = req.optLong("timeout_ms", 30_000L).coerceIn(500L, 120_000L)

        // First try last-known — instant if recent.
        val last = try { lm.getLastKnownLocation(provider) } catch (_: SecurityException) { null }
        val nowMs = System.currentTimeMillis()
        if (last != null && nowMs - last.time < req.optLong("max_age_ms", 30_000L)) {
            return RpcDispatcher.Response(toJson(last, provider, /*from_cache=*/ true))
        }

        // Otherwise, request a single update with a deadline.
        val ref = AtomicReference<Location?>(null)
        val lock = Object()
        val listener = object : LocationListener {
            override fun onLocationChanged(loc: Location) {
                synchronized(lock) { ref.set(loc); lock.notifyAll() }
            }
            override fun onProviderEnabled(p: String) {}
            override fun onProviderDisabled(p: String) {
                synchronized(lock) { lock.notifyAll() }
            }
        }
        val main = Looper.getMainLooper()
        try {
            lm.requestSingleUpdate(provider, listener, main)
        } catch (s: SecurityException) {
            return errResp("requestSingleUpdate denied: ${s.message}")
        }
        synchronized(lock) {
            val deadline = System.currentTimeMillis() + timeoutMs
            while (ref.get() == null && System.currentTimeMillis() < deadline) {
                lock.wait((deadline - System.currentTimeMillis()).coerceAtLeast(50L))
            }
        }
        try { lm.removeUpdates(listener) } catch (_: Throwable) {}
        val loc = ref.get()
            ?: return errResp("no fix within ${timeoutMs}ms (try outdoors or extend timeout)")
        return RpcDispatcher.Response(toJson(loc, provider, /*from_cache=*/ false))
    }

    private fun doStartStream(req: JSONObject): RpcDispatcher.Response {
        if (!hasFineLocPerm()) return errResp("ACCESS_FINE_LOCATION not granted")
        val streamId = req.optString("stream_id").ifBlank { "loc-${System.nanoTime()}" }
        if (activeStreams.containsKey(streamId)) {
            return errResp("stream '$streamId' already running; stop it first")
        }
        val provider = pickProvider(req.optString("provider", "gps"))
        if (!lm.isProviderEnabled(provider)) return errResp("provider '$provider' is disabled")
        val intervalMs = req.optLong("interval_ms", 5_000L).coerceAtLeast(500L)
        val minDist = req.optDouble("min_distance_m", 0.0).coerceAtLeast(0.0).toFloat()
        val store = EventStore.get(this)

        val listener = LocationListener { loc ->
            val data = JSONObject()
                .put("lat", loc.latitude)
                .put("lon", loc.longitude)
                .put("alt_m", if (loc.hasAltitude()) loc.altitude else JSONObject.NULL)
                .put("accuracy_m", if (loc.hasAccuracy()) loc.accuracy else JSONObject.NULL)
                .put("speed_mps", if (loc.hasSpeed()) loc.speed else JSONObject.NULL)
                .put("bearing_deg", if (loc.hasBearing()) loc.bearing else JSONObject.NULL)
                .put("provider", loc.provider ?: provider)
                .put("location_ts", loc.time)
            store.append("location", "gps_stream:$streamId", data)
        }
        try {
            lm.requestLocationUpdates(provider, intervalMs, minDist, listener, Looper.getMainLooper())
        } catch (s: SecurityException) {
            return errResp("requestLocationUpdates denied: ${s.message}")
        }
        activeStreams[streamId] = listener
        return RpcDispatcher.Response(JSONObject()
            .put("ok", true)
            .put("stream_id", streamId)
            .put("provider", provider)
            .put("interval_ms", intervalMs))
    }

    private fun doStopStream(req: JSONObject): RpcDispatcher.Response {
        val streamId = req.optString("stream_id")
        if (streamId.isBlank()) return errResp("missing 'stream_id'")
        val listener = activeStreams.remove(streamId)
            ?: return errResp("no active stream '$streamId'")
        try { lm.removeUpdates(listener) } catch (_: Throwable) {}
        return RpcDispatcher.Response(JSONObject().put("ok", true).put("stream_id", streamId))
    }

    private fun toJson(loc: Location, provider: String, from_cache: Boolean): JSONObject =
        JSONObject()
            .put("ok", true)
            .put("lat", loc.latitude)
            .put("lon", loc.longitude)
            .put("alt_m", if (loc.hasAltitude()) loc.altitude else JSONObject.NULL)
            .put("accuracy_m", if (loc.hasAccuracy()) loc.accuracy else JSONObject.NULL)
            .put("speed_mps", if (loc.hasSpeed()) loc.speed else JSONObject.NULL)
            .put("bearing_deg", if (loc.hasBearing()) loc.bearing else JSONObject.NULL)
            .put("provider", loc.provider ?: provider)
            .put("location_ts", loc.time)
            .put("from_cache", from_cache)

    private fun errResp(msg: String): RpcDispatcher.Response =
        RpcDispatcher.Response(JSONObject().put("ok", false).put("error", msg))

    private fun startForegroundNow() {
        val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            nm.createNotificationChannel(android.app.NotificationChannel(
                NOTIF_CHANNEL_ID, "Peko Location Bridge",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) })
        }
        val notif: Notification = NotificationCompat.Builder(this, NOTIF_CHANNEL_ID)
            .setContentTitle("Peko location bridge")
            .setContentText("GPS/fused location for the agent")
            .setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        activeStreams.values.forEach { try { lm.removeUpdates(it) } catch (_: Throwable) {} }
        activeStreams.clear()
        dispatcher.stop()
        super.onDestroy()
    }

    companion object {
        const val TAG = "PekoLocBridge"
        const val NOTIF_CHANNEL_ID = "peko_location_bridge"
        const val NOTIF_ID = 12
    }
}
