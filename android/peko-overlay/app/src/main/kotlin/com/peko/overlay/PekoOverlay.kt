package com.peko.overlay

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Root overlay composable. Two stacked states, swapped with a scale+fade
 * crossfade:
 *
 *   - Collapsed: a 64dp circular cat bubble. Draggable via WindowManager,
 *     tap expands, long-press dismisses the whole service.
 *   - Expanded: a 320x440 zinc-900 chat card with message list + composer.
 *
 * All state lives on [ChatController]; this file just reads it.
 */
@Composable
fun PekoOverlay(controller: ChatController) {
    MaterialTheme(colorScheme = PekoColors) {
        AnimatedContent(
            targetState = controller.isExpanded,
            transitionSpec = {
                (fadeIn() + scaleIn(initialScale = 0.85f)) togetherWith
                    (fadeOut() + scaleOut(targetScale = 0.85f))
            },
            label = "peko-overlay",
        ) { expanded ->
            if (expanded) ExpandedChat(controller) else CollapsedBubble(controller)
        }
    }
}

// ── Collapsed bubble ───────────────────────────────────────────────

@Composable
private fun CollapsedBubble(controller: ChatController) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.padding(4.dp),
    ) {
        controller.activityLabel?.let { label ->
            Surface(
                color = Color(0xFF27272A),
                contentColor = Color(0xFFE4E4E7),
                shape = RoundedCornerShape(12.dp),
                shadowElevation = 4.dp,
                modifier = Modifier.padding(bottom = 4.dp),
            ) {
                Text(
                    text = label,
                    fontSize = 11.sp,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                )
            }
        }

        Box(
            modifier = Modifier
                .size(64.dp)
                .clip(CircleShape)
                .background(Color(0xFF27272A))
                .border(1.5.dp, Color(0xFFA78BFA), CircleShape)
                .pointerInput(Unit) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        controller.drag(drag.x, drag.y)
                    }
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onTap = { controller.toggleExpanded() },
                        onLongPress = { controller.requestDismiss() },
                    )
                },
            contentAlignment = Alignment.Center,
        ) {
            Image(
                painter = rememberMascotPainter(controller.mascotPose),
                contentDescription = stringResource(R.string.app_name),
                modifier = Modifier.size(56.dp),
            )
        }
    }
}

// ── Expanded chat card ─────────────────────────────────────────────

@Composable
private fun ExpandedChat(controller: ChatController) {
    Surface(
        shape = RoundedCornerShape(20.dp),
        color = Color(0xFF18181B),
        contentColor = Color(0xFFE4E4E7),
        shadowElevation = 8.dp,
        modifier = Modifier
            .size(width = 320.dp, height = 440.dp)
            .border(1.dp, Color(0xFF7C3AED), RoundedCornerShape(20.dp)),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            ChatHeader(controller)
            ChatMessages(
                messages = controller.messages,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
            )
            ChatComposer(onSend = controller::send)
        }
    }
}

@Composable
private fun ChatHeader(controller: ChatController) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xFF27272A))
            .padding(horizontal = 12.dp, vertical = 8.dp)
            .pointerInput(Unit) {
                detectDragGestures { change, drag ->
                    change.consume()
                    controller.drag(drag.x, drag.y)
                }
            },
    ) {
        Image(
            painter = rememberMascotPainter(controller.mascotPose),
            contentDescription = null,
            modifier = Modifier.size(32.dp),
        )
        Spacer(Modifier.width(8.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = stringResource(R.string.mascot_greet_idle),
                color = Color(0xFFE4E4E7),
                fontWeight = FontWeight.SemiBold,
                fontSize = 14.sp,
            )
            controller.activityLabel?.let { label ->
                Text(text = label, color = Color(0xFFA1A1AA), fontSize = 11.sp)
            }
        }
        IconButton(onClick = { controller.toggleExpanded() }) {
            Icon(
                imageVector = Icons.Filled.Close,
                contentDescription = stringResource(R.string.chat_close),
                tint = Color(0xFFA1A1AA),
            )
        }
    }
}

@Composable
private fun ChatMessages(
    messages: List<ChatMessage>,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()
    LaunchedEffect(messages.size) {
        if (messages.isNotEmpty()) {
            listState.animateScrollToItem(messages.size - 1)
        }
    }
    if (messages.isEmpty()) {
        Box(modifier = modifier.padding(16.dp), contentAlignment = Alignment.Center) {
            Text(
                text = stringResource(R.string.mascot_greet_idle),
                color = Color(0xFFA1A1AA),
                fontSize = 12.sp,
            )
        }
        return
    }
    LazyColumn(
        state = listState,
        modifier = modifier.padding(horizontal = 10.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        items(count = messages.size, key = { it }) { i ->
            MessageBubble(messages[i])
        }
    }
}

@Composable
private fun MessageBubble(msg: ChatMessage) {
    when (msg.role) {
        ChatMessage.Role.User -> Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
        ) {
            Surface(
                color = Color(0xFF7C3AED),
                contentColor = Color.White,
                shape = RoundedCornerShape(14.dp, 14.dp, 4.dp, 14.dp),
            ) {
                Text(
                    text = msg.text,
                    fontSize = 13.sp,
                    modifier = Modifier
                        .widthIn(max = 240.dp)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                )
            }
        }

        ChatMessage.Role.Peko -> Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Start,
        ) {
            Surface(
                color = Color(0xFF27272A),
                contentColor = Color(0xFFE4E4E7),
                shape = RoundedCornerShape(14.dp, 14.dp, 14.dp, 4.dp),
            ) {
                Text(
                    text = msg.text.ifBlank { "…" },
                    fontSize = 13.sp,
                    modifier = Modifier
                        .widthIn(max = 240.dp)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                )
            }
        }

        ChatMessage.Role.Status -> Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
        ) {
            Surface(
                color = Color(0xFF27272A),
                contentColor = Color(0xFFA1A1AA),
                shape = RoundedCornerShape(999.dp),
            ) {
                Text(
                    text = msg.status ?: msg.text,
                    fontSize = 11.sp,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 3.dp),
                )
            }
        }
    }
}

@Composable
private fun ChatComposer(onSend: (String) -> Unit) {
    var draft by remember { mutableStateOf("") }
    val trySend: () -> Unit = {
        val t = draft.trim()
        if (t.isNotBlank()) {
            onSend(t)
            draft = ""
        }
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xFF27272A))
            .padding(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TextField(
            value = draft,
            onValueChange = { draft = it },
            placeholder = {
                Text(
                    text = stringResource(R.string.chat_hint),
                    color = Color(0xFFA1A1AA),
                    fontSize = 13.sp,
                )
            },
            singleLine = false,
            maxLines = 4,
            colors = TextFieldDefaults.colors(
                focusedContainerColor = Color(0xFF18181B),
                unfocusedContainerColor = Color(0xFF18181B),
                focusedIndicatorColor = Color.Transparent,
                unfocusedIndicatorColor = Color.Transparent,
                focusedTextColor = Color(0xFFE4E4E7),
                unfocusedTextColor = Color(0xFFE4E4E7),
                cursorColor = Color(0xFFA78BFA),
            ),
            shape = RoundedCornerShape(14.dp),
            modifier = Modifier.weight(1f),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { trySend() }),
        )
        Spacer(Modifier.width(6.dp))
        IconButton(
            onClick = trySend,
            modifier = Modifier
                .size(40.dp)
                .clip(CircleShape)
                .background(Color(0xFF7C3AED)),
        ) {
            // material-icons-core doesn't ship Send, so draw it as text.
            Text(
                text = "➤",
                color = Color.White,
                fontSize = 16.sp,
                fontWeight = FontWeight.Bold,
            )
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

@Composable
private fun rememberMascotPainter(pose: MascotPose): Painter = painterResource(
    when (pose) {
        MascotPose.Idle -> R.drawable.peko_cat
        MascotPose.Blink -> R.drawable.peko_cat_blink
        MascotPose.Thinking -> R.drawable.peko_cat_thinking
    },
)

private val PekoColors = darkColorScheme(
    primary = Color(0xFFA78BFA),
    onPrimary = Color.White,
    secondary = Color(0xFF7C3AED),
    onSecondary = Color.White,
    background = Color(0xFF18181B),
    onBackground = Color(0xFFE4E4E7),
    surface = Color(0xFF27272A),
    onSurface = Color(0xFFE4E4E7),
)
