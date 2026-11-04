package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.graphics.Bitmap
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.Display
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Current-call pixels only. The shared Rust tool path strips bytes before persistence. */
class PhoneScreenCapture(private val service: AccessibilityService) {
    private val main = Handler(Looper.getMainLooper())
    private val encoder = Executors.newSingleThreadExecutor()
    private var cancel: (() -> Unit)? = null

    fun capture(invoke: Invoke, observation: JSObject, valid: () -> Boolean, restoreOverlay: () -> Unit, recorded: (Int, Int, Int, Int) -> Unit) {
        if (Build.VERSION.SDK_INT < 30) { restoreOverlay(); invoke.reject("Screenshots require Android 11 or newer. Use the accessibility nodes on this device."); return }
        if (cancel != null) { restoreOverlay(); invoke.reject("A screenshot is already in progress."); return }
        val settled = AtomicBoolean(false)
        fun finish(error: String?, image: JSObject? = null) {
            if (!settled.compareAndSet(false, true)) return
            main.post {
                cancel = null; restoreOverlay()
                if (error != null) invoke.reject(error) else invoke.resolve(image ?: observation)
            }
        }
        val timeout = Runnable { finish("Android did not return a screenshot. Observe again before retrying.") }
        cancel = { finish("Screenshot cancelled because the accessibility service stopped.") }
        main.postDelayed(timeout, 8000)
        try {
            service.takeScreenshot(Display.DEFAULT_DISPLAY, service.mainExecutor, object : AccessibilityService.TakeScreenshotCallback {
                override fun onFailure(errorCode: Int) {
                    main.removeCallbacks(timeout)
                    finish("Android could not capture this screen (code $errorCode). Protected screens cannot be captured; use accessible controls or ask the user to continue.")
                }
                override fun onSuccess(result: AccessibilityService.ScreenshotResult) {
                    val buffer = result.hardwareBuffer
                    if (settled.get()) { buffer.close(); return }
                    val bitmap = try {
                        val hardware = Bitmap.wrapHardwareBuffer(buffer, result.colorSpace) ?: error("Screenshot pixels are unavailable.")
                        try { hardware.copy(Bitmap.Config.ARGB_8888, false) ?: error("Screenshot copy failed.") } finally { hardware.recycle() }
                    } catch (error: Exception) { finish(error.message ?: "Screenshot copy failed."); return } finally { buffer.close() }
                    if (!valid()) { bitmap.recycle(); finish("The active screen changed during capture. Observe again."); return }
                    try { encoder.execute {
                        var scaled: Bitmap? = null
                        try {
                            val ratio = minOf(1.0, 1280.0 / maxOf(bitmap.width, bitmap.height))
                            scaled = Bitmap.createScaledBitmap(bitmap, maxOf(1, (bitmap.width * ratio).toInt()), maxOf(1, (bitmap.height * ratio).toInt()), true)
                            val bytes = ByteArrayOutputStream().use { output ->
                                check(scaled!!.compress(Bitmap.CompressFormat.JPEG, 82, output)); output.toByteArray()
                            }
                            check(bytes.size <= 4 * 1024 * 1024) { "The screenshot is too large." }
                            val width = scaled!!.width; val height = scaled!!.height
                            val displayWidth = bitmap.width; val displayHeight = bitmap.height
                            val encoded = Base64.encodeToString(bytes, Base64.NO_WRAP)
                            main.post {
                                main.removeCallbacks(timeout)
                                if (!valid()) finish("The active screen changed during capture. Observe again.")
                                else if (!settled.get()) {
                                    recorded(width, height, displayWidth, displayHeight)
                                    observation.put("image", JSObject().apply {
                                        put("base64", encoded); put("mimeType", "image/jpeg"); put("width", width); put("height", height)
                                        put("displayWidth", displayWidth); put("displayHeight", displayHeight); put("coordinateSystem", "screenshot_pixels")
                                    })
                                    finish(null)
                                }
                            }
                        } catch (error: Exception) { finish(error.message ?: "Screenshot encoding failed.") }
                        finally { if (scaled !== bitmap) scaled?.recycle(); bitmap.recycle() }
                    } } catch (error: Exception) { bitmap.recycle(); finish("Screenshot service is closed.") }
                }
            })
        } catch (error: Exception) { main.removeCallbacks(timeout); finish(error.message ?: "Screenshot permission is unavailable.") }
    }
    fun close() { cancel?.invoke(); encoder.shutdownNow() }
}
