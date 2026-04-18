package com.peko.overlay

import android.content.Context
import android.view.View
import android.view.WindowManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/** One chat bubble shown in the overlay. */
data class ChatMessage(
    val role: Role,
    val text: String,
    /** Short activity label when role == Status, e.g. "opening youtube". */
    val status: String? = null,
) {
    enum class Role { User, Peko, Status }
}

/** High-level mascot animation state — drives which drawable is shown. */
enum class MascotPose { Idle, Blink, Thinking }

/**
 * Controller owned by [OverlayService]. Holds all mutable state the
 * Compose UI renders from, owns the coroutine scope that talks to
 * peko-agent, and knows how to move the WindowManager view when the
 * user drags the bubble.
 *
 * Keeping this outside the Composable tree makes lifecycle simple:
 * service creates one, passes it to the Composable, destroys it on teardown.
 */
class ChatController(
    private val appContext: Context,
    private val wm: WindowManager,
    private val view: View,
    private val lp: WindowManager.LayoutParams,
    private val onDismiss: () -> Unit,
) {
    // ── Observable state ───────────────────────────────────────

    var isExpanded by mutableStateOf(false)
        private set

    /** Live activity banner shown above the cat, e.g. "running tool: screenshot". */
    var activityLabel by mutableStateOf<String?>(null)
        private set

    var mascotPose by mutableStateOf(MascotPose.Idle)
        private set

    /** Chat history. Newest at the bottom so Compose LazyColumn can auto-scroll. */
    val messages = mutableStateListOf<ChatMessage>()

    /** Session id returned from peko-agent — stick to one so the agent keeps context. */
    private var sessionId: String? = null

    // ── Networking ─────────────────────────────────────────────

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val client = PekoClient(baseUrl = BASE_URL)

    // ── Idle blink loop ────────────────────────────────────────

    private var blinkJob: Job = scope.launch {
        while (true) {
            kotlinx.coroutines.delay(3_500)
            if (mascotPose == MascotPose.Idle) {
                mascotPose = MascotPose.Blink
                kotlinx.coroutines.delay(140)
                if (mascotPose == MascotPose.Blink) mascotPose = MascotPose.Idle
            }
        }
    }

    // ── Public actions (called from Composables) ──────────────

    fun toggleExpanded() {
        isExpanded = !isExpanded
        // When expanded we want the focusable bit on so the user can type.
        lp.flags = if (isExpanded) {
            lp.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE.inv()
        } else {
            lp.flags or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
        }
        runCatching { wm.updateViewLayout(view, lp) }
    }

    /** Drag handler — called from the bubble's pointerInput in Compose. */
    fun drag(dxPx: Float, dyPx: Float) {
        lp.x = (lp.x + dxPx).toInt().coerceIn(-2000, 3000)
        lp.y = (lp.y + dyPx).toInt().coerceIn(-500, 4000)
        runCatching { wm.updateViewLayout(view, lp) }
    }

    /** Long-press handler on the cat — stops the whole overlay service. */
    fun requestDismiss() = onDismiss()

    /** User sent a prompt. Fires the SSE request, appends deltas to the last Peko message. */
    fun send(prompt: String) {
        if (prompt.isBlank()) return
        messages.add(ChatMessage(ChatMessage.Role.User, prompt))
        val reply = ChatMessage(ChatMessage.Role.Peko, "")
        val replyIndex = messages.size
        messages.add(reply)
        mascotPose = MascotPose.Thinking
        activityLabel = appContext.getString(R.string.mascot_thinking)

        scope.launch {
            client.runTask(prompt, sessionId).collect { event ->
                when (event) {
                    is PekoEvent.TextDelta -> {
                        val cur = messages[replyIndex]
                        messages[replyIndex] = cur.copy(text = cur.text + event.text)
                    }
                    is PekoEvent.ToolStart -> {
                        activityLabel = "running ${event.name}"
                    }
                    is PekoEvent.ToolResult -> {
                        // Surface tool activity briefly; banner clears on Done.
                        val tag = if (event.isError) "error in" else "done"
                        activityLabel = "$tag ${event.name}"
                    }
                    is PekoEvent.Done -> {
                        sessionId = event.sessionId ?: sessionId
                        mascotPose = MascotPose.Idle
                        activityLabel = appContext.getString(R.string.mascot_done)
                        // Clear the banner after a short beat.
                        scope.launch {
                            kotlinx.coroutines.delay(2_500)
                            if (activityLabel == appContext.getString(R.string.mascot_done)) {
                                activityLabel = null
                            }
                        }
                    }
                    is PekoEvent.Error -> {
                        val cur = messages[replyIndex]
                        messages[replyIndex] = cur.copy(
                            text = if (cur.text.isBlank()) "Error: ${event.message}"
                                   else cur.text + "\n\nError: ${event.message}",
                        )
                        mascotPose = MascotPose.Idle
                        activityLabel = null
                    }
                    is PekoEvent.Thinking,
                    is PekoEvent.Status -> Unit  // surface later if useful
                }
            }
        }
    }

    fun stop() {
        blinkJob.cancel()
        scope.cancel()
    }

    companion object {
        /**
         * peko-agent always binds :8080 on localhost. App has its own network
         * namespace but loopback is still the phone. See
         * [network_security_config.xml] for the cleartext exception.
         */
        private const val BASE_URL = "http://127.0.0.1:8080"
    }
}
