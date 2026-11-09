package dev.zerohop.app

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.os.PersistableBundle
import android.os.SystemClock
import android.provider.OpenableColumns
import java.io.ByteArrayOutputStream
import java.io.FileNotFoundException
import java.io.IOException

private const val IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
private const val MAX_IMAGE = 24 * 1024 * 1024
private const val MAX_DECODED_PIXELS = 40_000_000L

/**
 * Reading happens only when the owner asks for it, because Android lets only
 * the app in front look at the clipboard. Writing can happen at any time.
 */
class SharedClipboard(
    private val context: Context,
    private val hasFocus: () -> Boolean,
) {
    private val board = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var serial = 0L
    private var applied: String? = null
    private var deadline = 0L
    private val expire = Runnable { settle() }

    fun read(done: (Map<String, Any?>) -> Unit) {
        val clip = board.primaryClip
        if (clip == null || clip.itemCount == 0) return done(state("empty"))
        // What arrived from another device carries the secret mark too, but
        // the owner should hear what is really going on.
        if (applied != null && clip.description.label?.toString() == applied) {
            return done(state("same"))
        }
        if (isSensitive(clip.description)) return done(state("secret"))
        val item = clip.getItemAt(0)
        val text = item.text?.toString()
        if (!text.isNullOrEmpty()) {
            return done(mapOf("state" to "ok", "format" to "text", "text" to text))
        }
        val uri = item.uri ?: return done(state("empty"))
        Thread {
            val result = readImage(uri)
            main.post { done(result) }
        }.start()
    }

    private fun readImage(uri: Uri): Map<String, Any?> {
        val resolver = context.contentResolver
        val type = resolver.getType(uri) ?: return state("empty")
        if (!type.startsWith("image/")) return state("empty")
        return try {
            val bytes = resolver.openInputStream(uri)?.use { input ->
                val out = ByteArrayOutputStream()
                val buffer = ByteArray(64 * 1024)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (out.size() + count > MAX_IMAGE) return state("too_large")
                    out.write(buffer, 0, count)
                }
                out.toByteArray()
            } ?: return state("empty")
            when (type) {
                "image/png" -> image("png", bytes)
                "image/jpeg" -> image("jpeg", bytes)
                else -> convertToPng(bytes)
            }
        } catch (_: IOException) {
            state("empty")
        } catch (_: SecurityException) {
            state("empty")
        }
    }

    private fun convertToPng(bytes: ByteArray): Map<String, Any?> {
        // A small file can claim to be an enormous picture.
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return state("empty")
        if (bounds.outWidth.toLong() * bounds.outHeight > MAX_DECODED_PIXELS) return state("too_large")
        return try {
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return state("empty")
            val out = ByteArrayOutputStream()
            bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out)
            bitmap.recycle()
            if (out.size() > MAX_IMAGE) state("too_large") else image("png", out.toByteArray())
        } catch (_: OutOfMemoryError) {
            state("too_large")
        }
    }

    private fun image(format: String, bytes: ByteArray): Map<String, Any?> =
        mapOf("state" to "ok", "format" to format, "data" to bytes)

    private fun state(name: String): Map<String, Any?> = mapOf("state" to name)

    private fun isSensitive(description: ClipDescription): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.N) return false
        return description.extras?.getBoolean(IS_SENSITIVE, false) == true
    }

    fun apply(format: String, text: String?, data: ByteArray?, ttlMs: Long) {
        serial += 1
        // How this app recognizes its own clip later: nothing else carries this label.
        val label = "Zerohop $serial"
        ClipImageProvider.wipe()
        val clip = if (format == "text") {
            ClipData.newPlainText(label, text ?: return)
        } else {
            val bytes = data ?: return
            val type = if (format == "jpeg") "image/jpeg" else "image/png"
            ClipImageProvider.keep(bytes, type)
            ClipData.newUri(context.contentResolver, label, ClipImageProvider.address(context, serial))
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            // The system's clipboard preview must not show this.
            clip.description.extras = PersistableBundle().apply { putBoolean(IS_SENSITIVE, true) }
        }
        board.setPrimaryClip(clip)
        applied = label
        deadline = SystemClock.elapsedRealtime() + ttlMs
        main.removeCallbacks(expire)
        main.postDelayed(expire, ttlMs)
    }

    /**
     * Removes an expired clip, but only if it is certainly still ours. Android
     * shows the clipboard only to the app in front, so while this app is in the
     * background the clip stays until the next time the owner opens it: wiping
     * blindly could destroy something copied since.
     */
    fun settle() {
        val label = applied ?: return
        if (SystemClock.elapsedRealtime() < deadline) return
        ClipImageProvider.wipe()
        if (!hasFocus()) return
        val current = board.primaryClipDescription?.label?.toString()
        if (current == label) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                board.clearPrimaryClip()
            } else {
                board.setPrimaryClip(ClipData.newPlainText("", ""))
            }
        }
        applied = null
    }
}

/** Serves a pasted picture from memory, so it is never written to storage. */
class ClipImageProvider : ContentProvider() {
    companion object {
        private val lock = Any()
        private var bytes: ByteArray? = null
        private var type = "image/png"

        fun keep(data: ByteArray, mime: String) = synchronized(lock) {
            bytes = data
            type = mime
        }

        fun wipe() = synchronized(lock) {
            bytes?.fill(0)
            bytes = null
        }

        fun address(context: Context, serial: Long): Uri =
            Uri.parse("content://${context.packageName}.clip/$serial")
    }

    override fun onCreate(): Boolean = true

    override fun getType(uri: Uri): String? = synchronized(lock) { if (bytes == null) null else type }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val data = synchronized(lock) { bytes?.copyOf() } ?: throw FileNotFoundException()
        if (mode != "r") throw FileNotFoundException()
        val (reader, writer) = ParcelFileDescriptor.createPipe()
        Thread {
            try {
                ParcelFileDescriptor.AutoCloseOutputStream(writer).use { it.write(data) }
            } catch (_: IOException) {
                // The app pasting it went away.
            } finally {
                data.fill(0)
            }
        }.start()
        return reader
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        val size = synchronized(lock) { bytes?.size } ?: throw FileNotFoundException()
        val extension = if (getType(uri) == "image/jpeg") "jpg" else "png"
        val cursor = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE))
        cursor.addRow(arrayOf<Any>("picture.$extension", size))
        return cursor
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = 0
}
