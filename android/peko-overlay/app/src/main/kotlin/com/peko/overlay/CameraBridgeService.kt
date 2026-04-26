package com.peko.overlay

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.ImageReader
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Log
import android.util.Size
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.peko.overlay.bridge.EventStore
import com.peko.overlay.bridge.RpcDispatcher
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.locks.ReentrantLock

/**
 * Camera2 bridge. Two flavours of operation:
 *
 *   capture { lens?:back|front, resolution?:"720p"|"1080p"|"max" }
 *     One-shot still. Opens device → builds JPEG ImageReader → fires a
 *     STILL_CAPTURE request → writes <out>/<id>.jpg → closes.
 *
 *   start_stream { stream_id?, lens?, fps?:1, max_frames?:0 (unlimited) }
 *     Continuous capture. Saves a JPEG per frame to
 *     /data/data/com.peko.overlay/files/vision/<ts>.jpg AND appends a
 *     "frame" event into the shared EventStore so the agent can pull
 *     latest frames via the events poller.
 *
 *   stop_stream { stream_id }
 *     Stop a running stream and release the camera.
 *
 * Single camera at a time — Camera2 doesn't allow multiple sessions on
 * the same device. start_stream while one is already running returns an
 * error.
 */
class CameraBridgeService : Service() {

    private lateinit var cm: CameraManager
    private lateinit var dispatcher: RpcDispatcher
    private val cameraLock = ReentrantLock()
    private var cameraThread: HandlerThread? = null
    private var cameraHandler: Handler? = null
    private val activeStream = AtomicReference<StreamState?>(null)
    private var visionDir: File? = null

    private data class StreamState(
        val streamId: String,
        val cameraDevice: CameraDevice,
        val session: CameraCaptureSession,
        val reader: ImageReader,
        val captureBuilder: CaptureRequest.Builder,
        val running: AtomicBoolean,
    )

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        cm = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        cameraThread = HandlerThread("PekoCam").also { it.start() }
        cameraHandler = Handler(cameraThread!!.looper)
        visionDir = File(filesDir, "vision").apply { mkdirs() }
        dispatcher = RpcDispatcher(this, "camera") { req, ctx -> handle(req, ctx) }
        dispatcher.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundNow()
        return START_STICKY
    }

    private fun handle(req: JSONObject, ctx: RpcDispatcher.RpcContext): RpcDispatcher.Response {
        return when (val action = req.optString("action")) {
            "capture" -> doCapture(req, ctx)
            "start_stream" -> doStartStream(req)
            "stop_stream" -> doStopStream(req)
            else -> err("unknown action '$action'")
        }
    }

    private fun hasPerm(): Boolean = ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
        PackageManager.PERMISSION_GRANTED

    private fun pickCameraId(lens: String): String? {
        val want = if (lens == "front") CameraCharacteristics.LENS_FACING_FRONT
        else CameraCharacteristics.LENS_FACING_BACK
        return cm.cameraIdList.firstOrNull { id ->
            cm.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING) == want
        } ?: cm.cameraIdList.firstOrNull()
    }

    private fun pickJpegSize(charact: CameraCharacteristics, want: String): Size {
        val map = charact.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return Size(1280, 720)
        val sizes = map.getOutputSizes(ImageFormat.JPEG) ?: emptyArray()
        val sorted = sizes.sortedByDescending { it.width.toLong() * it.height }
        return when (want) {
            "max" -> sorted.firstOrNull() ?: Size(1280, 720)
            "1080p" -> sorted.firstOrNull { it.height in 1000..1100 } ?: Size(1920, 1080)
            else -> sorted.firstOrNull { it.height in 700..760 } ?: Size(1280, 720)
        }
    }

    // ────── one-shot capture ──────────────────────────────────────

    private fun doCapture(req: JSONObject, ctx: RpcDispatcher.RpcContext): RpcDispatcher.Response {
        if (!hasPerm()) return err("CAMERA permission not granted")
        if (activeStream.get() != null) return err("camera busy: a stream is currently running, stop it first")

        val lens = req.optString("lens", "back")
        val cameraId = pickCameraId(lens) ?: return err("no camera matching lens=$lens")
        val charact = cm.getCameraCharacteristics(cameraId)
        val size = pickJpegSize(charact, req.optString("resolution", "1080p"))

        val reader = ImageReader.newInstance(size.width, size.height, ImageFormat.JPEG, 2)
        val outFile = File(ctx.outDir, "${ctx.id}.jpg")
        val captured = Semaphore(0)
        val errMsg = AtomicReference<String?>(null)
        reader.setOnImageAvailableListener({ r ->
            r.acquireLatestImage()?.use { img ->
                val buf = img.planes[0].buffer
                val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                try { outFile.writeBytes(bytes) }
                catch (t: Throwable) { errMsg.set("write jpeg: $t") }
                captured.release()
            }
        }, cameraHandler)

        val deviceRef = AtomicReference<CameraDevice?>(null)
        val sessionRef = AtomicReference<CameraCaptureSession?>(null)
        val deviceOpened = Semaphore(0)
        try {
            cm.openCamera(cameraId, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) { deviceRef.set(camera); deviceOpened.release() }
                override fun onDisconnected(camera: CameraDevice) {
                    camera.close(); errMsg.set("camera disconnected"); deviceOpened.release(); captured.release()
                }
                override fun onError(camera: CameraDevice, error: Int) {
                    camera.close(); errMsg.set("openCamera onError code=$error"); deviceOpened.release(); captured.release()
                }
            }, cameraHandler)
        } catch (s: SecurityException) {
            return err("openCamera denied: ${s.message}")
        }
        if (!deviceOpened.tryAcquire(5, TimeUnit.SECONDS)) return err("camera open timed out")
        val device = deviceRef.get() ?: return err(errMsg.get() ?: "camera failed to open")

        try {
            val builder = device.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE)
            builder.addTarget(reader.surface)
            builder.set(CaptureRequest.JPEG_ORIENTATION, 90)
            builder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
            builder.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_AUTO)

            val sessionDone = Semaphore(0)
            val cb = object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) {
                    sessionRef.set(s); sessionDone.release()
                }
                override fun onConfigureFailed(s: CameraCaptureSession) {
                    errMsg.set("session configure failed"); sessionDone.release()
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                val cfg = SessionConfiguration(
                    SessionConfiguration.SESSION_REGULAR,
                    listOf(OutputConfiguration(reader.surface)),
                    Executors.newSingleThreadExecutor(), cb,
                )
                device.createCaptureSession(cfg)
            } else {
                @Suppress("DEPRECATION")
                device.createCaptureSession(listOf(reader.surface), cb, cameraHandler)
            }
            if (!sessionDone.tryAcquire(5, TimeUnit.SECONDS)) return err("session configure timeout")
            val session = sessionRef.get() ?: return err(errMsg.get() ?: "session not ready")
            session.capture(builder.build(), null, cameraHandler)

            if (!captured.tryAcquire(8, TimeUnit.SECONDS)) return err("capture timed out")
            errMsg.get()?.let { return err(it) }
        } finally {
            try { sessionRef.get()?.close() } catch (_: Throwable) {}
            try { device.close() } catch (_: Throwable) {}
            try { reader.close() } catch (_: Throwable) {}
        }

        val resp = JSONObject()
            .put("ok", true)
            .put("width", size.width)
            .put("height", size.height)
            .put("size_bytes", outFile.length())
            .put("lens", lens)
        // outFile already lives at <outDir>/<id>.jpg — don't pass
        // assetSrc, the dispatcher would self-copy and ENOENT.
        return RpcDispatcher.Response(resp)
    }

    // ────── streaming capture ─────────────────────────────────────

    private fun doStartStream(req: JSONObject): RpcDispatcher.Response {
        if (!hasPerm()) return err("CAMERA permission not granted")
        if (activeStream.get() != null) return err("a stream is already running; stop it first")

        val streamId = req.optString("stream_id").ifBlank { "cam-${System.nanoTime()}" }
        val lens = req.optString("lens", "back")
        val fps = req.optDouble("fps", 1.0).coerceIn(0.1, 5.0) // capped to keep storage sane
        val maxFrames = req.optLong("max_frames", 0L) // 0 = unlimited
        val cameraId = pickCameraId(lens) ?: return err("no camera matching lens=$lens")
        val charact = cm.getCameraCharacteristics(cameraId)
        val size = pickJpegSize(charact, req.optString("resolution", "720p"))
        val reader = ImageReader.newInstance(size.width, size.height, ImageFormat.JPEG, 4)
        val running = AtomicBoolean(true)
        val frameCount = java.util.concurrent.atomic.AtomicLong(0)

        // Open camera + create session synchronously, then schedule
        // periodic captures from cameraHandler.
        val deviceRef = AtomicReference<CameraDevice?>(null)
        val deviceOpened = Semaphore(0)
        val errMsg = AtomicReference<String?>(null)
        try {
            cm.openCamera(cameraId, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) { deviceRef.set(camera); deviceOpened.release() }
                override fun onDisconnected(camera: CameraDevice) {
                    camera.close(); errMsg.set("disconnected"); deviceOpened.release()
                }
                override fun onError(camera: CameraDevice, error: Int) {
                    camera.close(); errMsg.set("onError $error"); deviceOpened.release()
                }
            }, cameraHandler)
        } catch (s: SecurityException) {
            return err("openCamera denied: ${s.message}")
        }
        if (!deviceOpened.tryAcquire(5, TimeUnit.SECONDS)) return err("camera open timed out")
        val device = deviceRef.get() ?: return err(errMsg.get() ?: "open failed")

        val sessionRef = AtomicReference<CameraCaptureSession?>(null)
        val sessionDone = Semaphore(0)
        val sessionCb = object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(s: CameraCaptureSession) { sessionRef.set(s); sessionDone.release() }
            override fun onConfigureFailed(s: CameraCaptureSession) { errMsg.set("session failed"); sessionDone.release() }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val cfg = SessionConfiguration(
                SessionConfiguration.SESSION_REGULAR,
                listOf(OutputConfiguration(reader.surface)),
                Executors.newSingleThreadExecutor(), sessionCb,
            )
            device.createCaptureSession(cfg)
        } else {
            @Suppress("DEPRECATION")
            device.createCaptureSession(listOf(reader.surface), sessionCb, cameraHandler)
        }
        if (!sessionDone.tryAcquire(5, TimeUnit.SECONDS)) {
            try { device.close() } catch (_: Throwable) {}
            try { reader.close() } catch (_: Throwable) {}
            return err("session configure timeout")
        }
        val session = sessionRef.get() ?: run {
            try { device.close() } catch (_: Throwable) {}
            try { reader.close() } catch (_: Throwable) {}
            return err(errMsg.get() ?: "session not ready")
        }

        val builder = device.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE).apply {
            addTarget(reader.surface)
            set(CaptureRequest.JPEG_ORIENTATION, 90)
            set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_AUTO)
        }

        val store = EventStore.get(this)
        reader.setOnImageAvailableListener({ r ->
            r.acquireLatestImage()?.use { img ->
                if (!running.get()) return@use
                val ts = System.currentTimeMillis()
                val buf = img.planes[0].buffer
                val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                val outFile = File(visionDir, "${ts}_$streamId.jpg")
                try { outFile.writeBytes(bytes) } catch (_: Throwable) { return@use }
                val data = JSONObject()
                    .put("width", size.width)
                    .put("height", size.height)
                    .put("lens", lens)
                    .put("size_bytes", bytes.size.toLong())
                store.append("frame", "camera_stream:$streamId", data, outFile.absolutePath)
                frameCount.incrementAndGet()
            }
            if (maxFrames > 0 && frameCount.get() >= maxFrames) {
                running.set(false)
                stopStreamInternal()
            }
        }, cameraHandler)

        // Schedule periodic capture requests on the camera handler.
        val intervalMs = (1000.0 / fps).toLong().coerceAtLeast(200L)
        val ticker = object : Runnable {
            override fun run() {
                if (!running.get()) return
                try { session.capture(builder.build(), null, cameraHandler) } catch (_: Throwable) {}
                cameraHandler?.postDelayed(this, intervalMs)
            }
        }
        cameraHandler?.post(ticker)

        activeStream.set(StreamState(streamId, device, session, reader, builder, running))
        return RpcDispatcher.Response(JSONObject()
            .put("ok", true)
            .put("stream_id", streamId)
            .put("lens", lens)
            .put("width", size.width)
            .put("height", size.height)
            .put("fps", fps))
    }

    private fun doStopStream(req: JSONObject): RpcDispatcher.Response {
        val want = req.optString("stream_id")
        val s = activeStream.get() ?: return err("no active stream")
        if (want.isNotBlank() && want != s.streamId) return err("active stream is '${s.streamId}', not '$want'")
        stopStreamInternal()
        return RpcDispatcher.Response(JSONObject()
            .put("ok", true)
            .put("stream_id", s.streamId))
    }

    private fun stopStreamInternal() {
        val s = activeStream.getAndSet(null) ?: return
        s.running.set(false)
        try { s.session.close() } catch (_: Throwable) {}
        try { s.cameraDevice.close() } catch (_: Throwable) {}
        try { s.reader.close() } catch (_: Throwable) {}
    }

    private fun err(msg: String): RpcDispatcher.Response =
        RpcDispatcher.Response(JSONObject().put("ok", false).put("error", msg))

    private fun startForegroundNow() {
        val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            nm.createNotificationChannel(android.app.NotificationChannel(
                NOTIF_CHANNEL_ID, "Peko Camera Bridge",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) })
        }
        val notif: Notification = NotificationCompat.Builder(this, NOTIF_CHANNEL_ID)
            .setContentTitle("Peko camera bridge")
            .setContentText("Camera2 capture for the agent")
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        stopStreamInternal()
        dispatcher.stop()
        cameraThread?.quitSafely()
        super.onDestroy()
    }

    companion object {
        const val TAG = "PekoCamBridge"
        const val NOTIF_CHANNEL_ID = "peko_camera_bridge"
        const val NOTIF_ID = 14
    }
}
