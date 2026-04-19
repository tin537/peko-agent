package com.peko.overlay

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.sse.EventSource
import okhttp3.sse.EventSourceListener
import okhttp3.sse.EventSources
import java.util.concurrent.TimeUnit

/**
 * Normalised event shape emitted by [PekoClient.runTask]. Mirrors the JSON
 * variants produced by `src/web/api.rs::run_task` so the Composables don't
 * need to know wire format.
 */
sealed interface PekoEvent {
    data class Status(val message: String) : PekoEvent
    data class TextDelta(val text: String) : PekoEvent
    data class Thinking(val text: String) : PekoEvent
    data class ToolStart(val name: String) : PekoEvent
    data class ToolResult(val name: String, val content: String, val isError: Boolean) : PekoEvent
    data class Done(val iterations: Int?, val sessionId: String?, val brain: String?) : PekoEvent
    data class Error(val message: String) : PekoEvent
}

/**
 * Thin wrapper around `POST /api/run` that turns its SSE response into a
 * cold Flow<PekoEvent>. The flow completes when the server emits the
 * sentinel `[DONE]` line or closes the stream for any reason.
 *
 * Uses OkHttp-SSE rather than hand-rolled parsing: it already handles
 * event-stream framing, reconnection-id state, and backpressure. Each
 * runTask() call gets its own EventSource so multiple concurrent chats
 * would Just Work (we don't do that yet).
 */
class PekoClient(private val baseUrl: String) {

    private val http = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)    // SSE is long-lived
        .connectTimeout(8, TimeUnit.SECONDS)
        .build()

    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    fun runTask(prompt: String, sessionId: String?): Flow<PekoEvent> = callbackFlow {
        // Build request body via kotlinx.serialization's JSON DSL — safer than
        // hand-rolling escapes for user-supplied prompt text.
        val body = buildJsonObject {
            put("input", prompt)
            if (sessionId != null) put("session_id", sessionId)
        }.toString()

        val req = Request.Builder()
            .url("$baseUrl/api/run")
            .post(body.toRequestBody(JSON_MEDIA))
            .header("Accept", "text/event-stream")
            .build()

        val listener = object : EventSourceListener() {
            override fun onEvent(es: EventSource, id: String?, type: String?, data: String) {
                if (data == "[DONE]") { close(); return }
                parseEvent(data)?.let { trySend(it) }
            }

            override fun onFailure(es: EventSource, t: Throwable?, response: Response?) {
                val msg = t?.message ?: response?.let { "HTTP ${it.code}" } ?: "stream closed"
                trySend(PekoEvent.Error(msg))
                close()
            }

            override fun onClosed(es: EventSource) { close() }
        }

        val es = EventSources.createFactory(http).newEventSource(req, listener)
        awaitClose { es.cancel() }
    }

    private fun parseEvent(rawJson: String): PekoEvent? {
        val obj = runCatching { json.parseToJsonElement(rawJson).jsonObject }.getOrNull() ?: return null
        val type = obj["type"]?.jsonPrimitive?.content ?: return null
        return when (type) {
            "status"      -> PekoEvent.Status(obj["message"]?.jsonPrimitive?.content ?: "")
            "text_delta"  -> PekoEvent.TextDelta(obj["text"]?.jsonPrimitive?.content ?: "")
            "thinking"    -> PekoEvent.Thinking(obj["text"]?.jsonPrimitive?.content ?: "")
            "tool_start"  -> PekoEvent.ToolStart(obj["name"]?.jsonPrimitive?.content ?: "?")
            "tool_result" -> PekoEvent.ToolResult(
                name    = obj["name"]?.jsonPrimitive?.content ?: "?",
                content = obj["content"]?.jsonPrimitive?.content ?: "",
                // Server sends is_error as a JSON boolean, so prefer the
                // typed accessor — .content would stringify "true"/"false"
                // and hide a malformed payload.
                isError = obj["is_error"]?.jsonPrimitive?.booleanOrNull ?: false,
            )
            "done"        -> PekoEvent.Done(
                // contentOrNull / intOrNull treat JSON null properly; plain
                // .content would surface it as the literal string "null".
                iterations = obj["iterations"]?.jsonPrimitive?.intOrNull,
                sessionId  = obj["session_id"]?.jsonPrimitive?.contentOrNull,
                brain      = obj["brain"]?.jsonPrimitive?.contentOrNull,
            )
            "error"       -> PekoEvent.Error(obj["message"]?.jsonPrimitive?.content ?: "unknown")
            else -> null
        }
    }

    companion object {
        private val JSON_MEDIA = "application/json; charset=utf-8".toMediaType()
    }
}
