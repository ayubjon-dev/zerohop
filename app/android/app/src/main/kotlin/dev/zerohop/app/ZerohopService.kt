package dev.zerohop.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.os.Build
import android.os.IBinder

private const val CHANNEL_ID = "background"
private const val NOTIFICATION_ID = 1

private fun backgroundNotification(context: Context): Notification {
    val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    val open = PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), flags)
    val send = PendingIntent.getActivity(
        context,
        1,
        Intent(context, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        flags,
    )
    val restore = PendingIntent.getBroadcast(
        context,
        2,
        Intent(context, NotificationSwipedReceiver::class.java),
        flags,
    )
    val action = Notification.Action.Builder(
        Icon.createWithResource(context, R.drawable.ic_stat_zerohop),
        context.getString(R.string.notification_send),
        send,
    ).build()
    val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        createChannel(context)
        Notification.Builder(context, CHANNEL_ID)
    } else {
        @Suppress("DEPRECATION")
        Notification.Builder(context)
    }
    return builder
        .setSmallIcon(R.drawable.ic_stat_zerohop)
        .setContentTitle(context.getString(R.string.notification_title))
        .setContentText(context.getString(R.string.notification_text))
        .setContentIntent(open)
        .setDeleteIntent(restore)
        .setOngoing(true)
        .setShowWhen(false)
        .addAction(action)
        .build()
}

private fun createChannel(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(
        CHANNEL_ID,
        context.getString(R.string.notification_channel),
        NotificationManager.IMPORTANCE_LOW,
    )
    channel.setShowBadge(false)
    context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
}

/** Keeps the app able to receive while nothing of it is on the screen. */
class ZerohopService : Service() {
    override fun onCreate() {
        super.onCreate()
        promote()
        (application as ZerohopApplication).engine()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        promote()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun promote() {
        val notification = backgroundNotification(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    companion object {
        fun start(context: Context) {
            val intent = Intent(context, ZerohopService::class.java)
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
            } catch (_: IllegalStateException) {
                // Not allowed from the background. The next launch starts it.
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ZerohopService::class.java))
        }
    }
}

/**
 * From Android 14 any notification can be swiped away, and with it the send
 * button. A swipe is easy to do by accident, so it comes straight back. Turning
 * the clipboard off, or Zerohop's notifications in Android settings, still works.
 */
class NotificationSwipedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val prefs = context.getSharedPreferences(BACKGROUND_PREFS, Context.MODE_PRIVATE)
        if (!prefs.getBoolean(BACKGROUND_ON, false)) return
        context.getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, backgroundNotification(context))
    }
}

/** Brings the service back after a restart or an update, if the owner wanted it. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED &&
            intent.action != Intent.ACTION_MY_PACKAGE_REPLACED
        ) {
            return
        }
        val prefs = context.getSharedPreferences(BACKGROUND_PREFS, Context.MODE_PRIVATE)
        if (prefs.getBoolean(BACKGROUND_ON, false)) ZerohopService.start(context)
    }
}
