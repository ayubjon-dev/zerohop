package dev.zerohop.app

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.StatFs
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.view.Gravity
import android.webkit.MimeTypeMap
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import java.io.File
import java.io.IOException
import java.util.UUID

private const val SHARED_DIR = "shared"
private const val MAX_FILES = 200
private const val MAX_NAME_BYTES = 200
private const val MIN_FREE_BYTES = 200L * 1024 * 1024
private const val SWEEP_AFTER_MS = 24L * 60 * 60 * 1000

/** Copying stopped because the phone is running out of room. */
private class NoRoomException : IOException()

/**
 * Takes what the owner shares from another app. Any app can open this, but
 * nothing leaves the phone from here: the files are shown in Zerohop and only
 * the owner's tap on Send moves them. Names and addresses come from the other
 * app and are treated as hostile.
 */
class ShareActivity : Activity() {
    @Volatile
    private var cancelled = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        try {
            handle()
        } catch (_: RuntimeException) {
            // A malformed request must not take the background service down with it.
            sayClip(application, "empty")
            finish()
        }
    }

    override fun onDestroy() {
        cancelled = true
        super.onDestroy()
    }

    private fun handle() {
        val streams = sharedStreams().filter(::isOtherAppsContent).take(MAX_FILES)
        if (streams.isNotEmpty()) {
            showProgress()
            Thread { copyAll(streams) }.start()
            return
        }
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        val app = application as ZerohopApplication
        when {
            text.isNullOrEmpty() -> sayClip(app, "empty")
            !textEntryEnabled() -> Toast.makeText(app, R.string.share_text_off, Toast.LENGTH_LONG).show()
            else -> app.sendClip(mapOf("state" to "ok", "format" to "text", "text" to text)) {
                sayClip(app, it)
            }
        }
        finish()
    }

    private fun textEntryEnabled(): Boolean {
        val entry = ComponentName(this, ClipShareActivity::class.java)
        return packageManager.getComponentEnabledSetting(entry) ==
            PackageManager.COMPONENT_ENABLED_STATE_ENABLED
    }

    private fun sharedStreams(): List<Uri> =
        when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(oneStream())
            Intent.ACTION_SEND_MULTIPLE -> manyStreams()
            else -> emptyList()
        }

    private fun oneStream(): Uri? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra(Intent.EXTRA_STREAM)
        }

    private fun manyStreams(): List<Uri> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
        }.orEmpty()

    /**
     * Whatever is named here is opened with this app's rights, so it may only
     * name another app's content, never a file and never something of this app's.
     */
    private fun isOtherAppsContent(uri: Uri): Boolean =
        uri.scheme == "content" && uri.authority?.startsWith(packageName) == false

    private fun showProgress() {
        val dark = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
            Configuration.UI_MODE_NIGHT_YES
        val density = resources.displayMetrics.density
        val card = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            val padding = (20 * density).toInt()
            setPadding(padding, padding, padding, padding)
            background = GradientDrawable().apply {
                setColor(if (dark) Color.rgb(30, 33, 40) else Color.WHITE)
                cornerRadius = 20 * density
            }
            addView(ProgressBar(context))
            addView(
                TextView(context).apply {
                    setText(R.string.share_preparing)
                    textSize = 16f
                    setTextColor(if (dark) Color.WHITE else Color.rgb(20, 22, 28))
                    setPadding((16 * density).toInt(), 0, 0, 0)
                },
            )
        }
        val root = LinearLayout(this).apply {
            gravity = Gravity.CENTER
            setBackgroundColor(Color.argb(90, 0, 0, 0))
            addView(card)
        }
        setContentView(root)
    }

    private fun copyAll(streams: List<Uri>) {
        val root = File(cacheDir, SHARED_DIR)
        sweep(root)
        val folders = mutableListOf<File>()
        val paths = mutableListOf<String>()
        val failure = try {
            for (uri in streams) {
                val folder = File(root, UUID.randomUUID().toString())
                if (!folder.mkdirs()) throw IOException("could not create a folder")
                folders += folder
                paths += copyOne(uri, folder).path
            }
            null
        } catch (_: NoRoomException) {
            R.string.share_no_room
        } catch (_: IOException) {
            R.string.share_failed
        } catch (_: RuntimeException) {
            R.string.share_failed
        }
        runOnUiThread { finishCopy(failure, folders, paths) }
    }

    private fun finishCopy(failure: Int?, folders: List<File>, paths: List<String>) {
        when {
            cancelled || failure != null -> {
                folders.forEach { it.deleteRecursively() }
                failure?.let { Toast.makeText(application, it, Toast.LENGTH_LONG).show() }
            }
            else -> {
                (application as ZerohopApplication).shareFiles(paths)
                startActivity(
                    Intent(this, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                )
            }
        }
        finish()
    }

    private fun copyOne(uri: Uri, folder: File): File {
        val target = File(folder, safeFileName(displayName(uri), fallbackName(uri)))
        if (!target.canonicalPath.startsWith(folder.canonicalPath + File.separator)) {
            throw IOException("a name pointed outside its folder")
        }
        val input = contentResolver.openInputStream(uri) ?: throw IOException("nothing to read")
        input.use { source ->
            target.outputStream().use { sink ->
                val buffer = ByteArray(256 * 1024)
                var sinceCheck = 0L
                while (true) {
                    if (cancelled) throw IOException("cancelled")
                    val count = source.read(buffer)
                    if (count < 0) break
                    sink.write(buffer, 0, count)
                    sinceCheck += count
                    // A hostile app could stream forever and fill the phone.
                    if (sinceCheck >= CHECK_ROOM_EVERY) {
                        sinceCheck = 0
                        if (StatFs(cacheDir.path).availableBytes < MIN_FREE_BYTES) throw NoRoomException()
                    }
                }
            }
        }
        modifiedTime(uri)?.let(target::setLastModified)
        return target
    }

    private fun displayName(uri: Uri): String? =
        queryOne(uri, OpenableColumns.DISPLAY_NAME) { it.first }

    private fun fallbackName(uri: Uri): String {
        val extension = contentResolver.getType(uri)?.let {
            MimeTypeMap.getSingleton().getExtensionFromMimeType(it)
        }
        return if (extension == null) "shared" else "shared.$extension"
    }

    private fun modifiedTime(uri: Uri): Long? {
        val document = DocumentsContract.Document.COLUMN_LAST_MODIFIED
        return queryOne(uri, document) { it.second }
            ?: queryOne(uri, MediaStore.MediaColumns.DATE_MODIFIED) { pair -> pair.second?.times(1000) }
    }

    private fun <T> queryOne(uri: Uri, column: String, pick: (Pair<String?, Long?>) -> T?): T? =
        try {
            contentResolver.query(uri, arrayOf(column), null, null, null)?.use { cursor ->
                if (!cursor.moveToFirst() || cursor.isNull(0)) {
                    null
                } else {
                    val text = cursor.getString(0)
                    pick(text to text?.toLongOrNull())
                }
            }
        } catch (_: RuntimeException) {
            null
        }

    /** Files shared but never sent do not stay around. */
    private fun sweep(root: File) {
        val oldest = System.currentTimeMillis() - SWEEP_AFTER_MS
        root.listFiles()?.filter { it.lastModified() < oldest }?.forEach { it.deleteRecursively() }
    }

    private companion object {
        const val CHECK_ROOM_EVERY = 16L * 1024 * 1024
    }
}

/** A name that stays inside its folder and fits on the file system. */
fun safeFileName(raw: String?, fallback: String): String {
    var name = (raw ?: "")
        .substringAfterLast('/')
        .substringAfterLast('\\')
        .filterNot { it.isISOControl() }
        .trim()
    while (name.toByteArray().size > MAX_NAME_BYTES) name = name.drop(1)
    return if (name.isEmpty() || name == "." || name == "..") fallback else name
}
