package com.peko.overlay

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.PixelFormat
import android.os.Build
import android.os.IBinder
import android.view.Gravity
import android.view.WindowManager
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.ComposeView
import androidx.core.app.NotificationCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.ServiceLifecycleDispatcher
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import androidx.lifecycle.LifecycleService

/**
 * Foreground service that owns the floating overlay. Lives for the life of
 * the user's session — NOT tied to any Activity. Boot flow:
 *
 *   onStartCommand → startForeground(ongoing notification)
 *                 → build ComposeView with [PekoOverlay] content
 *                 → attach the ComposeView to WindowManager with TYPE_APPLICATION_OVERLAY
 *
 *   onDestroy      → tear the view down, release coroutines inside PekoClient.
 *
 * Hosting a ComposeView outside an Activity requires plumbing the ViewTree
 * lifecycle/savedState/viewmodel owners manually — without them Compose's
 * animation APIs crash on first frame.
 */
class OverlayService : LifecycleService(), ViewModelStoreOwner, SavedStateRegistryOwner {

    private val vmStore = ViewModelStore()
    override val viewModelStore: ViewModelStore get() = vmStore

    private val savedStateController = SavedStateRegistryController.create(this)
    override val savedStateRegistry: SavedStateRegistry get() = savedStateController.savedStateRegistry

    private var wm: WindowManager? = null
    private var composeView: ComposeView? = null
    private var chatController: ChatController? = null

    override fun onCreate() {
        super.onCreate()
        // Restore savedState bits BEFORE Compose touches them.
        savedStateController.performAttach()
        savedStateController.performRestore(null)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (composeView != null) return START_STICKY  // already running
        startForegroundCompat()
        showOverlay()
        return START_STICKY
    }

    override fun onBind(intent: Intent): IBinder? {
        super.onBind(intent)
        return null   // unbound service; we use startForeground only
    }

    override fun onDestroy() {
        chatController?.stop()
        chatController = null
        composeView?.let { view ->
            runCatching { wm?.removeView(view) }
            view.disposeComposition()
        }
        composeView = null
        vmStore.clear()
        super.onDestroy()
    }

    // ── Overlay setup ───────────────────────────────────────────

    private fun showOverlay() {
        val wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        this.wm = wm

        val layerType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        } else {
            @Suppress("DEPRECATION")
            WindowManager.LayoutParams.TYPE_PHONE
        }

        val lp = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            layerType,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE    // no IME yet
                    or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
                    or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = 32
            y = 260
        }

        val view = ComposeView(this).apply {
            setViewTreeLifecycleOwner(this@OverlayService)
            setViewTreeViewModelStoreOwner(this@OverlayService)
            setViewTreeSavedStateRegistryOwner(this@OverlayService)
        }

        val controller = ChatController(applicationContext, wm, view, lp) {
            // When user dismisses with long-press, stop the whole service.
            stopSelf()
        }
        chatController = controller

        view.setContent {
            PekoOverlay(controller = controller)
        }

        wm.addView(view, lp)
        composeView = view
    }

    private fun startForegroundCompat() {
        val openIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pi = PendingIntent.getActivity(
            this, 0, openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification: Notification = NotificationCompat
            .Builder(this, PekoOverlayApp.NOTIF_CHANNEL_ID)
            .setSmallIcon(R.drawable.peko_cat)
            .setContentTitle(getString(R.string.notif_title))
            .setContentText(getString(R.string.notif_body))
            .setContentIntent(pi)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                PekoOverlayApp.NOTIF_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(PekoOverlayApp.NOTIF_ID, notification)
        }
    }
}
