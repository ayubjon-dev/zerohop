package dev.zerohop.app

import android.Manifest
import android.app.Activity
import android.app.Application
import android.app.DownloadManager
import android.app.StatusBarManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Icon
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.view.WindowManager
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.function.Consumer

const val BACKGROUND_PREFS = "background"
const val BACKGROUND_ON = "on"

/**
 * Owns the Flutter engine, so that the app keeps working with no screen open.
 * Activities only attach to it for as long as they are visible.
 */
class ZerohopApplication : Application(), Application.ActivityLifecycleCallbacks {
    val clipboard by lazy { SharedClipboard(this) { resumed?.hasWindowFocus() == true } }

    private val main = Handler(Looper.getMainLooper())
    private var flutter: FlutterEngine? = null
    private var channel: MethodChannel? = null
    private var resumed: Activity? = null
    private var ready = false
    private val waiting = ArrayDeque<() -> Unit>()
    private var awake = false
    private var askedNotifications = false
    private var scanning: MethodChannel.Result? = null
    private val wakeLock by lazy {
        getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "zerohop:transfer")
    }

    override fun onCreate() {
        super.onCreate()
        registerActivityLifecycleCallbacks(this)
    }

    fun engine(): FlutterEngine = flutter ?: start()

    private fun start(): FlutterEngine {
        val engine = FlutterEngine(this)
        channel = MethodChannel(engine.dartExecutor.binaryMessenger, "dev.zerohop/platform")
            .also { it.setMethodCallHandler(::handle) }
        engine.dartExecutor.executeDartEntrypoint(DartExecutor.DartEntrypoint.createDefault())
        flutter = engine
        return engine
    }

    /** Offers something copied here to the owner's other devices. */
    fun sendClip(copied: Map<String, Any?>, done: (String) -> Unit) {
        engine()
        val send = {
            channel?.invokeMethod(
                "clipboardSend",
                copied,
                object : MethodChannel.Result {
                    override fun success(result: Any?) = done(result as? String ?: "failed")

                    override fun error(code: String, message: String?, details: Any?) = done("failed")

                    override fun notImplemented() = done("failed")
                },
            )
            Unit
        }
        if (ready) send() else waiting.addLast(send)
    }

    /** Hands files shared from another app to the interface. */
    fun shareFiles(paths: List<String>) {
        engine()
        val send = {
            channel?.invokeMethod("sharedFiles", mapOf("paths" to paths))
            Unit
        }
        if (ready) send() else waiting.addLast(send)
    }

    private fun handle(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "deviceSecret" -> Thread {
                val secret = KeyVault.secret(this)
                main.post {
                    result.success(secret)
                    secret?.fill(0)
                }
            }.start()
            "downloads" -> result.success(receiveBase())
            "deviceName" -> result.success(deviceName())
            "show" -> {
                startActivity(
                    Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
                result.success(null)
            }
            "announce" -> {
                // Files moved out of the hidden working folder are on
                // disk, but other apps only see what the system indexed.
                val path = call.argument<String>("path")
                if (path != null) {
                    MediaScannerConnection.scanFile(this, arrayOf(path), null, null)
                }
                result.success(null)
            }
            "keepAwake" -> {
                keepAwake(call.argument<Boolean>("on") == true)
                result.success(null)
            }
            "scanQr" -> scan(call, result)
            "clipboardReady" -> {
                ready = true
                while (waiting.isNotEmpty()) waiting.removeFirst().invoke()
                result.success(null)
            }
            "clipboardRead" -> clipboard.read { result.success(it) }
            "clipboardApply" -> {
                clipboard.apply(
                    call.argument<String>("format") ?: "text",
                    call.argument<String>("text"),
                    call.argument<ByteArray>("data"),
                    (call.argument<Number>("ttl_ms") ?: 120_000).toLong(),
                )
                result.success(null)
            }
            "background" -> {
                setBackground(call.argument<Boolean>("on") == true)
                result.success(null)
            }
            "askNotifications" -> {
                askNotifications()
                result.success(null)
            }
            "addTile" -> addTile(result)
            "menuEntry" -> {
                val state = if (call.argument<Boolean>("on") == true) {
                    PackageManager.COMPONENT_ENABLED_STATE_ENABLED
                } else {
                    PackageManager.COMPONENT_ENABLED_STATE_DISABLED
                }
                packageManager.setComponentEnabledSetting(
                    ComponentName(this, ClipShareActivity::class.java),
                    state,
                    PackageManager.DONT_KILL_APP,
                )
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    private fun setBackground(on: Boolean) {
        getSharedPreferences(BACKGROUND_PREFS, MODE_PRIVATE)
            .edit()
            .putBoolean(BACKGROUND_ON, on)
            .apply()
        if (on) ZerohopService.start(this) else ZerohopService.stop(this)
    }

    /** Without this the notification and its button stay hidden on Android 13 and later. */
    private fun askNotifications() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val permission = Manifest.permission.POST_NOTIFICATIONS
        if (checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) return
        (resumed as? MainActivity)?.requestPermissions(arrayOf(permission), NOTIFICATIONS_REQUEST)
    }

    fun askNotificationsOnce() {
        if (askedNotifications) return
        askedNotifications = true
        askNotifications()
    }

    private fun addTile(result: MethodChannel.Result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return result.success(false)
        getSystemService(StatusBarManager::class.java).requestAddTileService(
            ComponentName(this, ClipTileService::class.java),
            getString(R.string.tile_label),
            Icon.createWithResource(this, R.drawable.ic_stat_zerohop),
            mainExecutor,
            Consumer<Int> { code ->
                result.success(
                    code == StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED ||
                        code == StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED,
                )
            },
        )
    }

    private fun keepAwake(on: Boolean) {
        awake = on
        applyScreenFlag()
        if (on && !wakeLock.isHeld) {
            wakeLock.acquire(MAX_TRANSFER_WAKE_MS)
        } else if (!on && wakeLock.isHeld) {
            wakeLock.release()
        }
    }

    private fun applyScreenFlag() {
        val window = (resumed as? MainActivity)?.window ?: return
        if (awake) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    /**
     * From Android 11 an app may create files in the shared Download folder
     * without any permission. Older systems only allow its own folder.
     */
    private fun receiveBase(): String {
        val shared: File? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                @Suppress("DEPRECATION")
                Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            } else {
                getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
            }
        return (shared ?: filesDir).absolutePath
    }

    private fun scan(call: MethodCall, result: MethodChannel.Result) {
        val from = resumed
        if (scanning != null || from == null) {
            result.error("unavailable", null, null)
            return
        }
        scanning = result
        val words = buildMap {
            for (key in listOf("title", "notZerohop", "cancel")) {
                call.argument<String>(key)?.let { put(key, it) }
            }
        }
        from.startActivity(ScanActivity.intent(from, words))
    }

    /** What the scanner saw: a pairing link, null for a cancel, or an error. */
    fun scanned(code: String?, error: String?) {
        val result = scanning ?: return
        scanning = null
        if (error != null) result.error(error, null, null) else result.success(code)
    }

    private fun deviceName(): String {
        val model = Build.MODEL ?: ""
        val maker = Build.MANUFACTURER?.replaceFirstChar { it.uppercase() } ?: ""
        return if (model.startsWith(maker, ignoreCase = true)) model else "$maker $model".trim()
    }

    override fun onActivityResumed(activity: Activity) {
        resumed = activity
        applyScreenFlag()
    }

    override fun onActivityPaused(activity: Activity) {
        if (resumed === activity) resumed = null
    }

    override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit

    override fun onActivityStarted(activity: Activity) = Unit

    override fun onActivityStopped(activity: Activity) = Unit

    override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit

    override fun onActivityDestroyed(activity: Activity) = Unit

    companion object {
        const val NOTIFICATIONS_REQUEST = 1
        private const val MAX_TRANSFER_WAKE_MS = 2 * 60 * 60 * 1000L
    }
}
