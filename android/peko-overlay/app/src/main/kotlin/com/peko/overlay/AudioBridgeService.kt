package com.peko.overlay

import android.app.Notification
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import com.peko.overlay.bridge.EventStore
import android.os.Build
import android.os.FileObserver
import android.os.IBinder
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.util.Log
import androidx.core.app.NotificationCompat
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlin.concurrent.thread

/**
 * Phase 5 — PCM record + playback bridge for peko-agent.
 *
 * Why a service inside the overlay APK, not direct ALSA from the Rust
 * agent: stock Android's audioserver owns `/dev/snd/pcmC*D*c`, so opening
 * those nodes from peko-agent (running as root) gets EBUSY. The
 * AudioRecord / AudioTrack Java APIs talk to audioserver via binder,
 * which only an Android app context can do. This service is the bridge.
 *
 * Protocol (file-based, mirrors [CallRecorderService] precedent):
 *
 *   peko-agent (root) writes:
 *     /data/data/com.peko.overlay/files/audio/in/<id>.json    — request
 *     /data/data/com.peko.overlay/files/audio/in/<id>.wav     — input PCM (play_wav only)
 *     /data/data/com.peko.overlay/files/audio/in/<id>.start   — sentinel; service picks up
 *
 *   service writes:
 *     /data/data/com.peko.overlay/files/audio/out/<id>.wav    — output PCM (record/tts)
 *     /data/data/com.peko.overlay/files/audio/out/<id>.json   — result metadata
 *     /data/data/com.peko.overlay/files/audio/out/<id>.done   — sentinel; agent picks up
 *
 * Sentinel rename is the atomic-handoff: agent writes .json + .wav fully
 * THEN creates an empty .start; service writes .wav + .json fully THEN
 * creates .done. Either side reads only after sentinel, so neither sees
 * a half-written file.
 *
 * Three actions:
 *   - record { duration_ms, sample_rate?:16000, channels?:1, source?:"mic" }
 *     → records mic audio, returns 16-bit LE PCM WAV.
 *   - play_wav {} (input WAV in .wav file)
 *     → plays input WAV via AudioTrack on STREAM_MUSIC.
 *   - tts { text, lang?:"en", rate?:1.0, pitch?:1.0 }
 *     → renders to a WAV via TextToSpeech.synthesizeToFile().
 */
class AudioBridgeService : Service() {

    private val executor = Executors.newSingleThreadExecutor()
    private val processed = ConcurrentHashMap<String, Boolean>()
    private var observer: FileObserver? = null
    private var inDir: File? = null
    private var outDir: File? = null
    private var tts: TextToSpeech? = null
    private var ttsReady = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        inDir = File(filesDir, "audio/in").apply { mkdirs() }
        outDir = File(filesDir, "audio/out").apply { mkdirs() }
        Log.i(TAG, "audio bridge starting; in=${inDir?.absolutePath} out=${outDir?.absolutePath}")

        // Lazy TTS init — completes async; tts() actions queue if !ready.
        tts = TextToSpeech(this) { status ->
            ttsReady = (status == TextToSpeech.SUCCESS)
            Log.i(TAG, "tts engine init status=$status ready=$ttsReady")
        }

        // Watch the in/ dir for new .start sentinels. FileObserver delivers
        // events on a dedicated thread; we shunt heavy work to executor.
        val dirPath = inDir!!.absolutePath
        observer = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            object : FileObserver(File(dirPath), CREATE or MOVED_TO or CLOSE_WRITE) {
                override fun onEvent(event: Int, path: String?) {
                    if (path != null && path.endsWith(".start")) onStart(path)
                }
            }
        } else {
            @Suppress("DEPRECATION")
            object : FileObserver(dirPath, CREATE or MOVED_TO or CLOSE_WRITE) {
                override fun onEvent(event: Int, path: String?) {
                    if (path != null && path.endsWith(".start")) onStart(path)
                }
            }
        }
        observer?.startWatching()

        // Pick up any requests that landed before we started watching
        // (boot-time race — agent might have queued a TTS/play during a
        // service restart). Cheap directory scan.
        inDir?.listFiles { f -> f.name.endsWith(".start") }?.forEach { onStart(it.name) }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundNow()
        return START_STICKY
    }

    private fun onStart(startName: String) {
        val id = startName.removeSuffix(".start")
        if (processed.putIfAbsent(id, true) != null) return
        executor.execute { processRequest(id) }
    }

    private fun processRequest(id: String) {
        val reqFile = File(inDir, "$id.json")
        val outJsonFile = File(outDir, "$id.json")
        val outWavFile = File(outDir, "$id.wav")
        val doneFile = File(outDir, "$id.done")
        try {
            if (!reqFile.exists()) {
                writeError(outJsonFile, doneFile, "request json missing")
                return
            }
            val req = JSONObject(reqFile.readText())
            val action = req.optString("action")
            val result = when (action) {
                "record" -> doRecord(req, outWavFile)
                "play_wav" -> doPlay(File(inDir, "$id.wav"))
                "tts" -> doTts(req, outWavFile)
                "route_get" -> doRouteGet()
                "route_set" -> doRouteSet(req)
                "start_ambient" -> doStartAmbient(req)
                "stop_ambient" -> doStopAmbient(req)
                else -> JSONObject().put("ok", false).put("error", "unknown action '$action'")
            }
            outJsonFile.writeText(result.toString())
            doneFile.writeText("")
        } catch (t: Throwable) {
            Log.e(TAG, "request $id failed", t)
            writeError(outJsonFile, doneFile, t.toString())
        } finally {
            // Clean inputs immediately so failed agents don't reprocess.
            File(inDir, "$id.start").delete()
            File(inDir, "$id.json").delete()
            File(inDir, "$id.wav").delete()
        }
    }

    private fun writeError(outJson: File, done: File, msg: String) {
        outJson.writeText(JSONObject().put("ok", false).put("error", msg).toString())
        done.writeText("")
    }

    // ────── record ─────────────────────────────────────────────────

    private fun doRecord(req: JSONObject, outWav: File): JSONObject {
        val durationMs = req.optInt("duration_ms", 5000).coerceIn(100, 120_000)
        val sampleRate = req.optInt("sample_rate", 16_000)
        val channels = req.optInt("channels", 1).coerceIn(1, 2)
        val srcStr = req.optString("source", "mic")
        val source = when (srcStr) {
            "voice_recognition" -> MediaRecorder.AudioSource.VOICE_RECOGNITION
            "voice_communication" -> MediaRecorder.AudioSource.VOICE_COMMUNICATION
            else -> MediaRecorder.AudioSource.MIC
        }
        val channelMask = if (channels == 1) AudioFormat.CHANNEL_IN_MONO else AudioFormat.CHANNEL_IN_STEREO
        val minBuf = AudioRecord.getMinBufferSize(sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT)
        if (minBuf <= 0) {
            return JSONObject().put("ok", false).put("error", "AudioRecord.getMinBufferSize failed for sr=$sampleRate ch=$channels")
        }
        val bufSize = (minBuf * 2).coerceAtLeast(sampleRate * channels * 2 / 4) // ~250ms

        val recorder = AudioRecord(source, sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT, bufSize)
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            recorder.release()
            return JSONObject().put("ok", false).put("error", "AudioRecord init failed (RECORD_AUDIO not granted? service.sh handles this on boot)")
        }
        val pcm = ByteArray((sampleRate * 2 * channels * durationMs / 1000.0).toInt())
        var pcmOffset = 0
        recorder.startRecording()
        val deadline = System.currentTimeMillis() + durationMs
        val chunk = ByteArray(bufSize)
        while (System.currentTimeMillis() < deadline && pcmOffset < pcm.size) {
            val n = recorder.read(chunk, 0, chunk.size)
            if (n <= 0) break
            val take = minOf(n, pcm.size - pcmOffset)
            System.arraycopy(chunk, 0, pcm, pcmOffset, take)
            pcmOffset += take
        }
        recorder.stop()
        recorder.release()
        val pcmTrimmed = pcm.copyOf(pcmOffset)
        writeWav(outWav, pcmTrimmed, sampleRate, channels, 16)
        return JSONObject()
            .put("ok", true)
            .put("duration_ms", durationMs)
            .put("sample_rate", sampleRate)
            .put("channels", channels)
            .put("size_bytes", outWav.length())
    }

    // ────── play_wav ───────────────────────────────────────────────

    private fun doPlay(inWav: File): JSONObject {
        if (!inWav.exists()) {
            return JSONObject().put("ok", false).put("error", "input WAV missing at ${inWav.path}")
        }
        val (pcm, sampleRate, channels, bitsPerSample) = readWav(inWav)
        if (bitsPerSample != 16) {
            return JSONObject().put("ok", false).put("error", "only 16-bit PCM supported, got $bitsPerSample")
        }
        val channelMask = if (channels == 1) AudioFormat.CHANNEL_OUT_MONO else AudioFormat.CHANNEL_OUT_STEREO
        val minBuf = AudioTrack.getMinBufferSize(sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT)
        if (minBuf <= 0) {
            return JSONObject().put("ok", false).put("error", "AudioTrack.getMinBufferSize failed sr=$sampleRate ch=$channels")
        }
        val track = AudioTrack.Builder()
            .setAudioAttributes(android.media.AudioAttributes.Builder()
                .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
                .setContentType(android.media.AudioAttributes.CONTENT_TYPE_SPEECH)
                .build())
            .setAudioFormat(AudioFormat.Builder()
                .setSampleRate(sampleRate)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setChannelMask(channelMask)
                .build())
            .setBufferSizeInBytes(minBuf.coerceAtLeast(pcm.size))
            .setTransferMode(AudioTrack.MODE_STATIC)
            .build()
        track.write(pcm, 0, pcm.size)
        track.play()
        // Block until done — duration ≈ samples / rate.
        val durationMs = (pcm.size.toLong() * 1000L) / (sampleRate.toLong() * channels.toLong() * 2L)
        Thread.sleep(durationMs + 100)
        track.stop()
        track.release()
        return JSONObject()
            .put("ok", true)
            .put("duration_ms", durationMs)
            .put("sample_rate", sampleRate)
            .put("channels", channels)
    }

    // ────── tts ─────────────────────────────────────────────────────

    private fun doTts(req: JSONObject, outWav: File): JSONObject {
        // Wait briefly for engine init.
        var waited = 0
        while (!ttsReady && waited < 5000) {
            Thread.sleep(100); waited += 100
        }
        if (!ttsReady) return JSONObject().put("ok", false).put("error", "TTS engine not ready after 5s")
        val text = req.optString("text", "").trim()
        if (text.isEmpty()) return JSONObject().put("ok", false).put("error", "text is empty")
        val lang = req.optString("lang", "en")
        tts?.language = Locale.forLanguageTag(lang)
        tts?.setSpeechRate(req.optDouble("rate", 1.0).toFloat())
        tts?.setPitch(req.optDouble("pitch", 1.0).toFloat())

        val utteranceId = "tts-${System.nanoTime()}"
        val done = Object()
        var failed: String? = null
        var success = false

        tts?.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) {}
            override fun onDone(utteranceId: String?) {
                synchronized(done) { success = true; done.notifyAll() }
            }
            @Deprecated("Deprecated in Java")
            override fun onError(utteranceId: String?) {
                synchronized(done) { failed = "tts onError"; done.notifyAll() }
            }
            override fun onError(utteranceId: String?, errorCode: Int) {
                synchronized(done) { failed = "tts onError code=$errorCode"; done.notifyAll() }
            }
        })

        val rc = tts?.synthesizeToFile(text, null, outWav, utteranceId) ?: TextToSpeech.ERROR
        if (rc != TextToSpeech.SUCCESS) {
            return JSONObject().put("ok", false).put("error", "synthesizeToFile rc=$rc")
        }
        synchronized(done) {
            if (!success && failed == null) {
                done.wait(30_000)
            }
        }
        if (failed != null) return JSONObject().put("ok", false).put("error", failed)
        if (!outWav.exists() || outWav.length() == 0L) {
            return JSONObject().put("ok", false).put("error", "tts produced no output file")
        }
        return JSONObject()
            .put("ok", true)
            .put("size_bytes", outWav.length())
            .put("text", text)
            .put("lang", lang)
    }

    // ────── route get/set ──────────────────────────────────────────

    private fun doRouteGet(): JSONObject {
        val am = getSystemService(AUDIO_SERVICE) as AudioManager
        val mode = when (am.mode) {
            AudioManager.MODE_NORMAL -> "NORMAL"
            AudioManager.MODE_IN_CALL -> "IN_CALL"
            AudioManager.MODE_IN_COMMUNICATION -> "IN_COMMUNICATION"
            AudioManager.MODE_RINGTONE -> "RINGTONE"
            else -> "MODE_${am.mode}"
        }
        return JSONObject()
            .put("ok", true)
            .put("mode", mode)
            .put("speaker", am.isSpeakerphoneOn)
            .put("bluetooth_sco", am.isBluetoothScoOn)
            .put("wired_headset_on", @Suppress("DEPRECATION") am.isWiredHeadsetOn)
            .put("music_active", am.isMusicActive)
            .put("volume_music",
                am.getStreamVolume(AudioManager.STREAM_MUSIC).toString() + "/" +
                    am.getStreamMaxVolume(AudioManager.STREAM_MUSIC))
            .put("volume_voice_call",
                am.getStreamVolume(AudioManager.STREAM_VOICE_CALL).toString() + "/" +
                    am.getStreamMaxVolume(AudioManager.STREAM_VOICE_CALL))
    }

    private fun doRouteSet(req: JSONObject): JSONObject {
        val am = getSystemService(AUDIO_SERVICE) as AudioManager
        if (req.has("mode")) {
            am.mode = when (req.optString("mode")) {
                "normal" -> AudioManager.MODE_NORMAL
                "in_call" -> AudioManager.MODE_IN_CALL
                "in_communication" -> AudioManager.MODE_IN_COMMUNICATION
                "ringtone" -> AudioManager.MODE_RINGTONE
                else -> return JSONObject().put("ok", false).put("error", "bad mode '${req.optString("mode")}'")
            }
        }
        if (req.has("speaker")) am.isSpeakerphoneOn = req.optBoolean("speaker")
        if (req.has("bluetooth_sco")) {
            if (req.optBoolean("bluetooth_sco")) am.startBluetoothSco() else am.stopBluetoothSco()
        }
        return doRouteGet()
    }

    // ────── ambient sound stream ──────────────────────────────────
    //
    // Captures 16kHz mono PCM in 1-second windows; for each window
    // emits an "ambient" event with low-cost features (RMS energy,
    // peak amplitude, zero-crossing rate). The shim does not classify
    // — that's deferred. The agent can poll events and ship interesting
    // windows to a downstream classifier (cloud Whisper, on-device
    // YAMNet later, etc.). This gives the agent ambient awareness
    // without bundling ML models in the APK.

    @Volatile private var ambientThread: Thread? = null
    @Volatile private var ambientStreamId: String? = null

    private fun doStartAmbient(req: JSONObject): JSONObject {
        if (ambientThread != null) {
            return JSONObject().put("ok", false).put("error", "ambient stream already running")
        }
        val streamId = req.optString("stream_id").ifBlank { "amb-${System.nanoTime()}" }
        val sampleRate = req.optInt("sample_rate", 16_000)
        val windowMs = req.optInt("window_ms", 1000).coerceIn(200, 5000)
        val minRms = req.optDouble("min_rms", 0.0).coerceAtLeast(0.0).toInt()
        val channelMask = AudioFormat.CHANNEL_IN_MONO
        val minBuf = AudioRecord.getMinBufferSize(sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT)
        if (minBuf <= 0) {
            return JSONObject().put("ok", false).put("error", "AudioRecord unsupported config")
        }
        val bufSize = (sampleRate * windowMs / 1000 * 2).coerceAtLeast(minBuf)
        val rec = AudioRecord(MediaRecorder.AudioSource.MIC, sampleRate, channelMask,
            AudioFormat.ENCODING_PCM_16BIT, bufSize)
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            return JSONObject().put("ok", false).put("error", "AudioRecord init failed (RECORD_AUDIO?)")
        }
        ambientStreamId = streamId
        val store = EventStore.get(this)
        val window = ShortArray(sampleRate * windowMs / 1000)
        ambientThread = thread(start = true, name = "PekoAmbient") {
            rec.startRecording()
            try {
                while (Thread.currentThread() == ambientThread && !Thread.currentThread().isInterrupted) {
                    val n = rec.read(window, 0, window.size)
                    if (n <= 0) continue
                    var sumSq = 0.0
                    var peak = 0
                    var zc = 0
                    var prev: Short = 0
                    for (i in 0 until n) {
                        val s = window[i].toInt()
                        sumSq += (s.toDouble() * s.toDouble())
                        val a = if (s < 0) -s else s
                        if (a > peak) peak = a
                        if (i > 0 && ((prev.toInt() >= 0) != (s >= 0))) zc++
                        prev = window[i]
                    }
                    val rms = kotlin.math.sqrt(sumSq / n).toInt()
                    if (rms < minRms) continue
                    val data = JSONObject()
                        .put("rms", rms)
                        .put("peak", peak)
                        .put("zc", zc)
                        .put("samples", n)
                        .put("sample_rate", sampleRate)
                        .put("window_ms", windowMs)
                    store.append("ambient", "audio_stream:$streamId", data)
                }
            } catch (_: Throwable) {
            } finally {
                try { rec.stop() } catch (_: Throwable) {}
                rec.release()
            }
        }
        return JSONObject()
            .put("ok", true)
            .put("stream_id", streamId)
            .put("sample_rate", sampleRate)
            .put("window_ms", windowMs)
    }

    private fun doStopAmbient(req: JSONObject): JSONObject {
        val want = req.optString("stream_id")
        val active = ambientStreamId
        if (active == null) return JSONObject().put("ok", false).put("error", "no active ambient stream")
        if (want.isNotBlank() && want != active) {
            return JSONObject().put("ok", false).put("error", "active stream is '$active', not '$want'")
        }
        val t = ambientThread
        ambientThread = null
        ambientStreamId = null
        try { t?.interrupt(); t?.join(2000) } catch (_: Throwable) {}
        return JSONObject().put("ok", true).put("stream_id", active)
    }

    // ────── WAV helpers (16-bit PCM, little-endian) ───────────────

    private fun writeWav(out: File, pcm: ByteArray, sampleRate: Int, channels: Int, bitsPerSample: Int) {
        val byteRate = sampleRate * channels * bitsPerSample / 8
        val blockAlign = channels * bitsPerSample / 8
        FileOutputStream(out).use { os ->
            val header = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN).apply {
                put("RIFF".toByteArray())
                putInt(36 + pcm.size)
                put("WAVE".toByteArray())
                put("fmt ".toByteArray())
                putInt(16)                 // PCM chunk size
                putShort(1)                // format = PCM
                putShort(channels.toShort())
                putInt(sampleRate)
                putInt(byteRate)
                putShort(blockAlign.toShort())
                putShort(bitsPerSample.toShort())
                put("data".toByteArray())
                putInt(pcm.size)
            }
            os.write(header.array())
            os.write(pcm)
        }
    }

    private data class WavSpec(val pcm: ByteArray, val sampleRate: Int, val channels: Int, val bitsPerSample: Int)

    private fun readWav(file: File): WavSpec {
        val bytes = file.readBytes()
        require(bytes.size >= 44 && String(bytes, 0, 4) == "RIFF" && String(bytes, 8, 4) == "WAVE") {
            "not a WAV file"
        }
        // Find 'fmt ' and 'data' chunks; tolerate optional chunks before data.
        var i = 12
        var sampleRate = 0; var channels = 0; var bitsPerSample = 0
        var dataOffset = -1; var dataLen = 0
        while (i + 8 <= bytes.size) {
            val id = String(bytes, i, 4)
            val size = ByteBuffer.wrap(bytes, i + 4, 4).order(ByteOrder.LITTLE_ENDIAN).int
            when (id) {
                "fmt " -> {
                    val buf = ByteBuffer.wrap(bytes, i + 8, size).order(ByteOrder.LITTLE_ENDIAN)
                    buf.short // format code
                    channels = buf.short.toInt()
                    sampleRate = buf.int
                    buf.int    // byte rate
                    buf.short  // block align
                    bitsPerSample = buf.short.toInt()
                }
                "data" -> {
                    dataOffset = i + 8
                    dataLen = size
                }
            }
            i += 8 + size
            if (dataOffset > 0) break
        }
        require(dataOffset > 0) { "WAV has no data chunk" }
        val pcm = bytes.copyOfRange(dataOffset, dataOffset + dataLen)
        return WavSpec(pcm, sampleRate, channels, bitsPerSample)
    }

    // ────── foreground notification ────────────────────────────────

    private fun startForegroundNow() {
        val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = android.app.NotificationChannel(
                NOTIF_CHANNEL_ID, "Peko Audio Bridge",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) }
            nm.createNotificationChannel(ch)
        }
        val notif: Notification = NotificationCompat.Builder(this, NOTIF_CHANNEL_ID)
            .setContentTitle("Peko audio bridge")
            .setContentText("Mic + speaker shim for the agent")
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            // Type=microphone is required on 14+ for AudioRecord to keep
            // working when the app's process is backgrounded. mediaPlayback
            // covers AudioTrack/TTS even when the screen is off.
            startForeground(
                NOTIF_ID, notif,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                    or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
            )
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        observer?.stopWatching(); observer = null
        try { tts?.stop(); tts?.shutdown() } catch (_: Throwable) {}
        executor.shutdownNow()
        super.onDestroy()
    }

    companion object {
        const val TAG = "PekoAudioBridge"
        const val NOTIF_CHANNEL_ID = "peko_audio_bridge"
        const val NOTIF_ID = 11
    }
}
