package com.peko.overlay

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat

/**
 * Two-state activity:
 *   - If overlay permission is missing → render a small consent screen
 *     (tap → system Settings → user grants → come back → activity finishes).
 *   - If overlay permission is granted → immediately start [OverlayService]
 *     and finish. The activity should not hang around after the overlay is up.
 *
 * We do NOT host the chat UI here. The chat lives in a WindowManager view
 * controlled by OverlayService so it outlives every other app on screen.
 */
class MainActivity : ComponentActivity() {

    private val postNotifLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) {
        // Ignore the result — notification perm isn't load-bearing. Service
        // starts regardless; user just won't see the FGS notification.
        launchOverlay()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        if (hasOverlayPermission()) {
            maybeRequestNotifThenLaunch()
            return
        }

        setContent {
            MaterialTheme(colorScheme = pekoDarkScheme) {
                PermissionScreen(
                    onGrant = {
                        val intent = Intent(
                            Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                            Uri.parse("package:$packageName"),
                        )
                        startActivity(intent)
                    },
                    onCheck = {
                        if (hasOverlayPermission()) maybeRequestNotifThenLaunch()
                    },
                )
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Coming back from Settings — check again. If the user granted the
        // permission, kick the service and disappear.
        if (hasOverlayPermission()) maybeRequestNotifThenLaunch()
    }

    private fun hasOverlayPermission(): Boolean = Settings.canDrawOverlays(this)

    private fun maybeRequestNotifThenLaunch() {
        // Android 13+ needs runtime POST_NOTIFICATIONS for the FGS notification.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val granted = ContextCompat.checkSelfPermission(
                this, android.Manifest.permission.POST_NOTIFICATIONS,
            ) == android.content.pm.PackageManager.PERMISSION_GRANTED
            if (!granted) {
                postNotifLauncher.launch(android.Manifest.permission.POST_NOTIFICATIONS)
                return
            }
        }
        launchOverlay()
    }

    private fun launchOverlay() {
        val intent = Intent(this, OverlayService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
        finish()
    }
}

@Composable
private fun PermissionScreen(onGrant: () -> Unit, onCheck: () -> Unit) {
    Scaffold(
        containerColor = Color(0xFF18181B),
    ) { pad ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .background(Color(0xFF18181B))
                .padding(pad)
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(
                painter = painterResource(R.drawable.peko_cat),
                contentDescription = null,
                modifier = Modifier.height(96.dp),
                tint = Color.Unspecified,
            )
            Spacer(Modifier.height(24.dp))
            Text(
                stringResource(R.string.perm_title),
                color = Color(0xFFE4E4E7),
                fontSize = 20.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(R.string.perm_body),
                color = Color(0xFFA1A1AA),
                fontSize = 14.sp,
            )
            Spacer(Modifier.height(24.dp))
            Button(
                onClick = onGrant,
                colors = ButtonDefaults.buttonColors(
                    containerColor = Color(0xFF7C3AED),
                    contentColor = Color.White,
                ),
            ) { Text(stringResource(R.string.perm_grant)) }
            Spacer(Modifier.height(8.dp))
            Button(
                onClick = onCheck,
                colors = ButtonDefaults.outlinedButtonColors(
                    containerColor = Color(0xFF27272A),
                    contentColor = Color(0xFFE4E4E7),
                ),
            ) { Text(stringResource(R.string.perm_retry)) }
        }
    }
}

private val pekoDarkScheme = darkColorScheme(
    primary = Color(0xFFA78BFA),
    onPrimary = Color.White,
    background = Color(0xFF18181B),
    surface = Color(0xFF27272A),
    onSurface = Color(0xFFE4E4E7),
)
