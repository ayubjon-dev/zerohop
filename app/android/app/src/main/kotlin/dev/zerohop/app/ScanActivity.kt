package dev.zerohop.app

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Reads the pairing QR code shown by another device. Frames are decoded on
 * this phone with ZXing; nothing leaves it. Only a Zerohop pairing code is
 * handed back, and any other code is ignored with a short note.
 */
class ScanActivity : ComponentActivity() {
    private val app get() = application as ZerohopApplication
    private lateinit var camera: ExecutorService
    private lateinit var hint: TextView
    private val reader = QRCodeReader()
    private val hints = mapOf(DecodeHintType.TRY_HARDER to true)
    private var done = false

    private val askCamera =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) startCamera() else finishWith(error = "camera_denied")
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        camera = Executors.newSingleThreadExecutor()
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
            finishWith(error = "no_camera")
            return
        }
        setContentView(layout())
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            startCamera()
        } else {
            askCamera.launch(Manifest.permission.CAMERA)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        camera.shutdown()
        // Closed by the system or the back gesture: the same as Cancel.
        finishWith(code = null)
    }

    private lateinit var preview: PreviewView

    private fun layout(): View {
        val root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
        preview = PreviewView(this).apply { scaleType = PreviewView.ScaleType.FILL_CENTER }
        root.addView(preview, FrameLayout.LayoutParams(-1, -1))
        root.addView(Shade(this), FrameLayout.LayoutParams(-1, -1))
        val title = label(intent.getStringExtra(EXTRA_TITLE), 18f).apply {
            setTypeface(typeface, android.graphics.Typeface.BOLD)
        }
        hint = label(intent.getStringExtra(EXTRA_NOT_ZEROHOP), 15f).apply { alpha = 0f }
        val cancel = Button(this).apply {
            text = intent.getStringExtra(EXTRA_CANCEL)
            isAllCaps = false
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
            background = GradientDrawable().apply {
                cornerRadius = dp(28f)
                setColor(Color.argb(46, 255, 255, 255))
            }
            setPadding(dp(28f).toInt(), 0, dp(28f).toInt(), 0)
            minHeight = dp(52f).toInt()
            setOnClickListener { finishWith(code = null) }
        }
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            addView(title)
        }
        val bottom = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            addView(hint)
            addView(
                cancel,
                LinearLayout.LayoutParams(-2, -2).apply { topMargin = dp(20f).toInt() },
            )
        }
        root.addView(top, FrameLayout.LayoutParams(-1, -2, Gravity.TOP))
        root.addView(bottom, FrameLayout.LayoutParams(-1, -2, Gravity.BOTTOM))
        // Edge to edge: keep the text clear of the status and navigation bars.
        ViewCompat.setOnApplyWindowInsetsListener(root) { _, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            val side = dp(24f).toInt()
            top.setPadding(side, bars.top + dp(32f).toInt(), side, 0)
            bottom.setPadding(side, 0, side, bars.bottom + dp(24f).toInt())
            insets
        }
        return root
    }

    private fun label(text: String?, size: Float) = TextView(this).apply {
        this.text = text
        setTextColor(Color.WHITE)
        setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
        gravity = Gravity.CENTER
    }

    private fun startCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            if (done) return@addListener
            try {
                val provider = future.get()
                val shown = Preview.Builder().build().also {
                    it.setSurfaceProvider(preview.surfaceProvider)
                }
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(camera, ::decode)
                val selector = if (provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA)) {
                    CameraSelector.DEFAULT_BACK_CAMERA
                } else {
                    CameraSelector.DEFAULT_FRONT_CAMERA
                }
                provider.unbindAll()
                provider.bindToLifecycle(this, selector, shown, analysis)
            } catch (e: Exception) {
                finishWith(error = "no_camera")
            }
        }, ContextCompat.getMainExecutor(this))
    }

    /** Reads the brightness plane of a frame, which is all a QR code needs. */
    private fun decode(image: ImageProxy) {
        try {
            val plane = image.planes[0]
            val buffer = plane.buffer
            val data = ByteArray(buffer.remaining()).also { buffer.get(it) }
            val source = PlanarYUVLuminanceSource(
                data, plane.rowStride, image.height, 0, 0, image.width, image.height, false,
            )
            val text = try {
                reader.decode(BinaryBitmap(HybridBinarizer(source)), hints).text
            } catch (_: ReaderException) {
                null
            } finally {
                reader.reset()
            }
            if (text != null) runOnUiThread { found(text) }
        } catch (_: RuntimeException) {
            // A frame that cannot be read is skipped; the next one may work.
        } finally {
            image.close()
        }
    }

    private fun found(text: String) {
        if (done) return
        if (text.startsWith(PREFIX)) {
            val buzz = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                HapticFeedbackConstants.CONFIRM
            } else {
                HapticFeedbackConstants.VIRTUAL_KEY
            }
            window.decorView.performHapticFeedback(buzz)
            finishWith(code = text)
            return
        }
        hint.animate().cancel()
        hint.alpha = 1f
        hint.animate().alpha(0f).setStartDelay(2000).setDuration(300).start()
    }

    /** Hands back the code, null for a cancel, or an error, exactly once. */
    private fun finishWith(code: String? = null, error: String? = null) {
        if (done) return
        done = true
        app.scanned(code, error)
        if (!isFinishing) finish()
    }

    private fun dp(value: Float) =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, resources.displayMetrics)

    /** Darkens everything but the square the owner aims at. */
    private class Shade(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(140, 0, 0, 0) }
        private val path = Path().apply { fillType = Path.FillType.EVEN_ODD }
        private val square = RectF()

        override fun onDraw(canvas: Canvas) {
            val side = minOf(width, height) * FRAME_SHARE
            square.set(
                (width - side) / 2, (height - side) / 2, (width + side) / 2, (height + side) / 2,
            )
            val radius = 24 * resources.displayMetrics.density
            path.reset()
            path.addRect(0f, 0f, width.toFloat(), height.toFloat(), Path.Direction.CW)
            path.addRoundRect(square, radius, radius, Path.Direction.CW)
            canvas.drawPath(path, paint)
        }
    }

    companion object {
        private const val PREFIX = "zerohop://pair/"
        private const val FRAME_SHARE = 0.7f
        private const val EXTRA_TITLE = "title"
        private const val EXTRA_NOT_ZEROHOP = "notZerohop"
        private const val EXTRA_CANCEL = "cancel"

        fun intent(context: Context, words: Map<String, String>) =
            Intent(context, ScanActivity::class.java).apply {
                putExtra(EXTRA_TITLE, words[EXTRA_TITLE])
                putExtra(EXTRA_NOT_ZEROHOP, words[EXTRA_NOT_ZEROHOP])
                putExtra(EXTRA_CANCEL, words[EXTRA_CANCEL])
            }
    }
}
