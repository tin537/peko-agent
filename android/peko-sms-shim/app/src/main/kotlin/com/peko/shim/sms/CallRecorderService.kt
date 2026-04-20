package com.peko.shim.sms

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.media.MediaRecorder
import android.media.ToneGenerator
import android.os.Build
import android.os.IBinder
import android.os.SystemClock
import android.util.Log
import java.io.File

/**
 * Foreground service that records the audio of an in-progress phone
 * call and writes it (plus metadata) to the shim's private files dir,
 * where peko-agent's CallPipeline picks it up for transcription and
 * summarisation.
 *
 * Why a foreground service, not a background one:
 *   - Android 12+ forbids starting non-foreground services from a
 *     BroadcastReceiver that isn't already in the foreground.
 *   - Recording audio from a microphone source requires
 *     FOREGROUND_SERVICE_MICROPHONE and a FGS type declaration on
 *     Android 14+; we set it on 13+ to stay forward-compatible.
 *
 * Output layout (polled by peko-agent via CallPipeline):
 *   /data/data/com.peko.shim.sms/files/calls/<id>.m4a    — audio
 *   /data/data/com.peko.shim.sms/files/calls/<id>.json   — metadata
 *   /data/data/com.peko.shim.sms/files/calls/<id>.done   — sentinel,
 *         created atomically after the m4a+json pair is fully flushed
 *
 * Sentinel (`.done`) pattern matches sms_out — prevents peko-agent
 * from picking up a half-finished recording mid-stop().
 *
 * Consent beeps: two short 440Hz-ish tones played on STREAM_VOICE_CALL
 * just before MediaRecorder starts. Both parties hear them (streamed
 * through the voice path) so the recording is not covert. Keep it
 * short — pre-answer beeps would be cut off by the connect event.
 */
class CallRecorderService : Service() {

    private var recorder: MediaRecorder? = null
    private var tone: ToneGenerator? = null
    private var startedAtMs: Long = 0
    private var startedAtWallMs: Long = 0
    private var callId: String? = null
    private var number: String? = null
    private var direction: String? = null
    private var audioFile: File? = null
    private var metaFile: File? = null
    private var doneFile: File? = null
    private var srcUsed: String = "unknown"

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                if (recorder != null) {
                    Log.w(TAG, "START received while already recording, ignoring")
                    return START_NOT_STICKY
                }
                startForegroundNow()
                val num = intent.getStringExtra(EXTRA_NUMBER) ?: ""
                val dir = intent.getStringExtra(EXTRA_DIRECTION) ?: "unknown"
                beginRecording(num, dir)
            }
            ACTION_STOP -> {
                endRecording()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
            else -> {
                Log.w(TAG, "unknown action ${intent?.action}")
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        // Defence-in-depth — endRecording() is normally called from
        // ACTION_STOP, but if the service is killed for any reason
        // (low-memory, user force-stop) we still want a valid file
        // on disk rather than a truncated one that breaks peko's
        // transcription pipeline.
        if (recorder != null) {
            endRecording()
        }
        super.onDestroy()
    }

    private fun startForegroundNow() {
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(
                CHANNEL_ID,
                "Peko call recording",
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = "Active while Peko is recording an in-progress phone call."
                setShowBadge(false)
            }
            nm.createNotificationChannel(ch)
        }
        val notif: Notification = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
                .setContentTitle("Peko is recording this call")
                .setContentText("Audio will be summarised for your records.")
                .setSmallIcon(android.R.drawable.stat_sys_speakerphone)
                .setOngoing(true)
                .setCategory(Notification.CATEGORY_CALL)
                .build()
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
                .setContentTitle("Peko is recording this call")
                .setSmallIcon(android.R.drawable.stat_sys_speakerphone)
                .setOngoing(true)
                .build()
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    private fun beginRecording(num: String, dir: String) {
        val id = "call-${System.currentTimeMillis()}-${(Math.random() * 1e6).toInt()}"
        callId = id
        number = num
        direction = dir
        startedAtWallMs = System.currentTimeMillis()
        startedAtMs = SystemClock.elapsedRealtime()

        val callsDir = File(filesDir, "calls")
        if (!callsDir.isDirectory && !callsDir.mkdirs()) {
            Log.e(TAG, "mkdir failed ${callsDir.absolutePath}")
            return
        }
        val audio = File(callsDir, "$id.m4a")
        val meta  = File(callsDir, "$id.json")
        val done  = File(callsDir, "$id.done")
        audioFile = audio
        metaFile  = meta
        doneFile  = done

        // Two-beep consent tone — both parties hear it via the
        // in-call audio path. Don't block for the full duration: use
        // the tone generator's own timing so we can kick off the
        // MediaRecorder promptly.
        playConsentBeeps()

        // Preferred source is VOICE_CALL (uplink+downlink mixed) —
        // needs CAPTURE_AUDIO_OUTPUT which we hold as a priv-app via
        // privapp-permissions XML. If the HAL refuses it (OEM
        // policy on some devices), fall back to VOICE_COMMUNICATION
        // which at least captures the local side over the handset
        // mic. MIC is the last resort for speakerphone calls.
        val candidates = listOf(
            "VOICE_CALL"          to MediaRecorder.AudioSource.VOICE_CALL,
            "VOICE_COMMUNICATION" to MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            "MIC"                 to MediaRecorder.AudioSource.MIC,
        )
        var lastErr: Throwable? = null
        for ((name, src) in candidates) {
            try {
                val r = newRecorder().apply {
                    setAudioSource(src)
                    setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
                    setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
                    setAudioSamplingRate(16_000)
                    setAudioEncodingBitRate(64_000)
                    setOutputFile(audio.absolutePath)
                    prepare()
                    start()
                }
                recorder = r
                srcUsed = name
                Log.i(TAG, "recording started id=$id src=$name file=${audio.absolutePath}")
                writeMetadata(partial = true)
                return
            } catch (e: Throwable) {
                lastErr = e
                Log.w(TAG, "source=$name refused: ${e.message}")
            }
        }
        Log.e(TAG, "all audio sources failed id=$id", lastErr)
        // Still write a metadata file so peko-agent can surface the
        // failed attempt in the Calls tab — otherwise this call
        // disappears silently.
        srcUsed = "none"
        writeMetadata(partial = false, error = lastErr?.message ?: "all sources refused")
        markDone()
    }

    private fun endRecording() {
        val r = recorder
        recorder = null
        try {
            r?.stop()
        } catch (e: Throwable) {
            // RuntimeException from MediaRecorder.stop() means the
            // recording was too short to have any encoded frames — the
            // output file ends up zero-length, which we note in meta.
            Log.w(TAG, "stop() failed: ${e.message}")
        }
        try { r?.reset() } catch (_: Throwable) {}
        try { r?.release() } catch (_: Throwable) {}
        try { tone?.release() } catch (_: Throwable) {}
        tone = null
        writeMetadata(partial = false)
        markDone()
    }

    private fun playConsentBeeps() {
        try {
            val t = ToneGenerator(AudioManager.STREAM_VOICE_CALL, 80)
            tone = t
            // TONE_PROP_BEEP is a clear short tone; play twice with
            // a gap. ToneGenerator returns immediately; durations are
            // in ms and handled internally.
            t.startTone(ToneGenerator.TONE_PROP_BEEP, 150)
            Thread.sleep(250)
            t.startTone(ToneGenerator.TONE_PROP_BEEP, 150)
            // Don't release here — we want the generator to finish
            // its second tone before we tear it down. endRecording()
            // releases it after the call ends.
        } catch (e: Throwable) {
            Log.w(TAG, "consent beep failed (non-fatal): ${e.message}")
        }
    }

    private fun newRecorder(): MediaRecorder {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            MediaRecorder(this)
        } else {
            @Suppress("DEPRECATION")
            MediaRecorder()
        }
    }

    private fun writeMetadata(partial: Boolean, error: String? = null) {
        val meta = metaFile ?: return
        val id = callId ?: return
        val durMs = if (startedAtMs == 0L) 0L else SystemClock.elapsedRealtime() - startedAtMs
        val sizeBytes = audioFile?.takeIf { it.exists() }?.length() ?: 0L
        val json = buildString {
            append('{')
            append("\"id\":\"").append(esc(id)).append("\",")
            append("\"direction\":\"").append(esc(direction ?: "unknown")).append("\",")
            append("\"number\":\"").append(esc(number ?: "")).append("\",")
            append("\"started_at_ms\":").append(startedAtWallMs).append(',')
            append("\"duration_ms\":").append(durMs).append(',')
            append("\"audio_src\":\"").append(esc(srcUsed)).append("\",")
            append("\"audio_path\":\"").append(esc(audioFile?.absolutePath ?: "")).append("\",")
            append("\"audio_bytes\":").append(sizeBytes).append(',')
            append("\"partial\":").append(partial)
            if (error != null) {
                append(",\"error\":\"").append(esc(error)).append("\"")
            }
            append('}')
        }
        val tmp = File(meta.parentFile, meta.name + ".tmp")
        try {
            tmp.writeText(json)
            if (!tmp.renameTo(meta)) {
                Log.w(TAG, "rename ${tmp} → ${meta} failed")
            }
        } catch (e: Throwable) {
            Log.e(TAG, "writeMetadata io error id=$id", e)
        }
    }

    private fun markDone() {
        val d = doneFile ?: return
        try {
            d.writeText(System.currentTimeMillis().toString())
        } catch (e: Throwable) {
            Log.w(TAG, "markDone failed", e)
        }
    }

    private fun esc(s: String): String {
        val sb = StringBuilder(s.length + 4)
        for (c in s) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"'  -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> if (c.code < 0x20) sb.append("\\u%04x".format(c.code)) else sb.append(c)
            }
        }
        return sb.toString()
    }

    companion object {
        private const val TAG = "PekoCallRec"
        private const val CHANNEL_ID = "peko_call_rec"
        private const val NOTIF_ID = 0x7ecb

        const val ACTION_START = "com.peko.shim.sms.CALL_REC_START"
        const val ACTION_STOP  = "com.peko.shim.sms.CALL_REC_STOP"

        const val EXTRA_NUMBER    = "number"
        const val EXTRA_DIRECTION = "direction"
    }
}
