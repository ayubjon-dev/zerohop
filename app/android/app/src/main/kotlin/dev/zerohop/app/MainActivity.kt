package dev.zerohop.app

import android.content.Context
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

class MainActivity : FlutterActivity() {
    private val app get() = application as ZerohopApplication

    // The engine belongs to the application and outlives this screen.
    override fun provideFlutterEngine(context: Context): FlutterEngine = app.engine()

    override fun onResume() {
        super.onResume()
        // For an owner who turned the clipboard on before this version existed.
        val prefs = getSharedPreferences(BACKGROUND_PREFS, MODE_PRIVATE)
        if (prefs.getBoolean(BACKGROUND_ON, false)) app.askNotificationsOnce()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) app.clipboard.settle()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == ZerohopApplication.NOTIFICATIONS_REQUEST) {
            // The notification posted before permission was given stays hidden.
            val prefs = getSharedPreferences(BACKGROUND_PREFS, MODE_PRIVATE)
            if (prefs.getBoolean(BACKGROUND_ON, false)) ZerohopService.start(this)
        }
    }
}
