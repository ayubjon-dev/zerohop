package dev.zerohop.app

import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast

/** Says briefly how sending went, so that no send goes by unnoticed. */
fun sayClip(context: Context, state: String) {
    val message = when (state) {
        "sent", "sending" -> R.string.clip_sent
        "off" -> R.string.clip_off
        "no_devices" -> R.string.clip_no_devices
        "secret" -> R.string.clip_secret
        "too_large" -> R.string.clip_too_large
        "same" -> R.string.clip_same
        "empty", "files" -> R.string.clip_empty
        else -> R.string.clip_failed
    }
    Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
}

/**
 * An invisible window. Android lets only the app in front read the clipboard,
 * so this comes forward for a moment, reads it, and leaves. It is not exported:
 * only this app's own notification and tile can start it.
 */
class ClipSendActivity : Activity() {
    private var started = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || started) return
        started = true
        val app = application as ZerohopApplication
        app.clipboard.settle()
        app.clipboard.read { copied ->
            val state = copied["state"] as? String ?: "empty"
            if (state == "ok") {
                app.sendClip(copied) { outcome -> sayClip(app, outcome) }
            } else {
                sayClip(app, state)
            }
            finish()
            @Suppress("DEPRECATION")
            overridePendingTransition(0, 0)
        }
    }
}

/**
 * The entry in the text selection menu. Any app in front can start this and
 * hand over anything it likes, so it stays disabled until the owner turns it on.
 */
class ClipShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = application as ZerohopApplication
        try {
            val text = intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
            if (text.isNullOrEmpty()) {
                sayClip(app, "empty")
            } else {
                app.sendClip(mapOf("state" to "ok", "format" to "text", "text" to text)) {
                    sayClip(app, it)
                }
            }
        } catch (_: RuntimeException) {
            // A malformed request must not take the background service down with it.
            sayClip(app, "empty")
        }
        finish()
    }
}

/** A button in the quick settings shade: sends what was just copied. */
class ClipTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        qsTile?.let {
            it.state = Tile.STATE_ACTIVE
            it.updateTile()
        }
    }

    override fun onClick() {
        super.onClick()
        val intent = Intent(this, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE),
            )
        } else {
            @Suppress("DEPRECATION", "StartActivityAndCollapseDeprecated")
            startActivityAndCollapse(intent)
        }
    }
}
