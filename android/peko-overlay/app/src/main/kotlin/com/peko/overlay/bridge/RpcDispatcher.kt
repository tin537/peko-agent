package com.peko.overlay.bridge

import android.content.Context
import android.os.Build
import android.os.FileObserver
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * Shared file-RPC dispatcher used by every bridge service (GPS,
 * telephony, camera, audio). Watches `<files>/<topic>/in/` for `.start`
 * sentinels, hands the request body to a Handler, writes the result
 * + sentinel back into `<files>/<topic>/out/`.
 *
 * Topics segregate request streams so a slow camera capture doesn't
 * block a GPS fix on the same FileObserver.
 *
 * Protocol (mirrors the established AudioBridgeService pattern):
 *
 *   peko-agent writes:
 *     <topic>/in/<id>.json             — request body
 *     <topic>/in/<id>.start            — sentinel; dispatcher picks up
 *
 *   handler writes (via Response):
 *     <topic>/out/<id>.json            — result body
 *     <topic>/out/<id>.<asset_ext>?    — optional binary asset
 *     <topic>/out/<id>.done            — sentinel; agent picks up
 */
class RpcDispatcher(
    private val ctx: Context,
    private val topic: String,
    private val handler: (JSONObject, RpcContext) -> Response,
) {
    data class Response(val body: JSONObject, val assetSrc: File? = null, val assetExt: String? = null)

    class RpcContext(val id: String, val inDir: File, val outDir: File)

    private val executor = Executors.newSingleThreadExecutor()
    private val processed = ConcurrentHashMap<String, Boolean>()
    private var observer: FileObserver? = null
    private val inDir: File = File(ctx.filesDir, "$topic/in").apply { mkdirs() }
    private val outDir: File = File(ctx.filesDir, "$topic/out").apply { mkdirs() }

    fun start() {
        Log.i(TAG, "$topic dispatcher starting; in=${inDir.path} out=${outDir.path}")
        val dirPath = inDir.absolutePath
        observer = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            object : FileObserver(File(dirPath), CREATE or MOVED_TO or CLOSE_WRITE) {
                override fun onEvent(event: Int, path: String?) {
                    if (path?.endsWith(".start") == true) onStart(path)
                }
            }
        } else {
            @Suppress("DEPRECATION")
            object : FileObserver(dirPath, CREATE or MOVED_TO or CLOSE_WRITE) {
                override fun onEvent(event: Int, path: String?) {
                    if (path?.endsWith(".start") == true) onStart(path)
                }
            }
        }
        observer?.startWatching()
        // Pick up requests that landed before the observer registered.
        inDir.listFiles { f -> f.name.endsWith(".start") }?.forEach { onStart(it.name) }
    }

    fun stop() {
        observer?.stopWatching(); observer = null
        executor.shutdownNow()
    }

    private fun onStart(startName: String) {
        val id = startName.removeSuffix(".start")
        if (processed.putIfAbsent(id, true) != null) return
        executor.execute { process(id) }
    }

    private fun process(id: String) {
        val req = File(inDir, "$id.json")
        val outJson = File(outDir, "$id.json")
        val done = File(outDir, "$id.done")
        try {
            if (!req.exists()) {
                writeError(outJson, done, "request json missing"); return
            }
            val body = JSONObject(req.readText())
            val resp = handler(body, RpcContext(id, inDir, outDir))
            // Write asset first, JSON second, sentinel last — same atomic-ish
            // ordering peko-agent expects.
            if (resp.assetSrc != null && resp.assetExt != null) {
                resp.assetSrc.copyTo(File(outDir, "$id.${resp.assetExt}"), overwrite = true)
            }
            outJson.writeText(resp.body.toString())
            done.writeText("")
        } catch (t: Throwable) {
            Log.e(TAG, "$topic request $id failed", t)
            writeError(outJson, done, t.toString())
        } finally {
            File(inDir, "$id.start").delete()
            File(inDir, "$id.json").delete()
            // Clean any input asset (e.g. play_wav input WAV).
            File(inDir, "$id.wav").delete()
        }
    }

    private fun writeError(outJson: File, done: File, msg: String) {
        outJson.writeText(JSONObject().put("ok", false).put("error", msg).toString())
        done.writeText("")
    }

    companion object { private const val TAG = "PekoRpc" }
}
