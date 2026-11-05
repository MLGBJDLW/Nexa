package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.Display
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.nio.ByteBuffer
import java.security.MessageDigest

data class PhoneCapture(val state: JSObject, val fingerprint: String, val width: Int, val height: Int, val displayWidth: Int, val displayHeight: Int)
class PhoneCaptureChanged(message: String) : IllegalStateException(message)

/** Current-call pixels only. The shared Rust tool path strips bytes before persistence. */
class PhoneScreenCapture(private val service: AccessibilityService) {
    private val main = Handler(Looper.getMainLooper())
    private val encoder = Executors.newSingleThreadExecutor()
    private var cancel: (() -> Unit)? = null
    private var lastCaptureAt = 0L

    fun capture(observation: JSObject, bounds: Rect, valid: () -> Boolean, complete: (Result<PhoneCapture>) -> Unit) {
        if (Build.VERSION.SDK_INT < 30) { complete(Result.failure(IllegalStateException("Screenshots require Android 11 or newer. Use accessibility nodes."))); return }
        if (cancel != null) { complete(Result.failure(IllegalStateException("A screenshot is already in progress."))); return }
        val appBounds = Rect(bounds)
        // Edge-to-edge app roots may cover the clock and navigation bar.
        // Exclude only system insets, retaining keyboard/content pixels.
        val metrics = service.getSystemService(android.view.WindowManager::class.java).maximumWindowMetrics
        val insets = metrics.windowInsets.getInsetsIgnoringVisibility(android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
        appBounds.intersect(metrics.bounds.left + insets.left, metrics.bounds.top + insets.top,
            metrics.bounds.right - insets.right, metrics.bounds.bottom - insets.bottom)
        val settled = AtomicBoolean(false)
        fun finish(error: String?, frame: PhoneCapture? = null, changed: Boolean = false) {
            if (!settled.compareAndSet(false, true)) return
            main.post {
                cancel = null
                if (error != null) complete(Result.failure(if (changed) PhoneCaptureChanged(error) else IllegalStateException(error))) else complete(Result.success(requireNotNull(frame)))
            }
        }
        val timeout = Runnable { finish("Android did not return a screenshot. Observe again before retrying.") }
        cancel = { finish("Screenshot cancelled because the accessibility service stopped.") }
        main.postDelayed(timeout, 8000)
        main.postDelayed({ try {
            if (settled.get()) return@postDelayed
            if (!valid()) { finish("The active screen changed before capture. Observe again.", changed = true); return@postDelayed }
            lastCaptureAt = android.os.SystemClock.elapsedRealtime()
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
                    if (!valid()) { bitmap.recycle(); finish("The active screen changed during capture. Observe again.", changed = true); return }
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
                            // Hash actual app pixels, excluding unrelated system bars.
                            // A model delay alone cannot invalidate unchanged content.
                            val left = (appBounds.left * ratio).toInt().coerceIn(0, width - 1)
                            val top = (appBounds.top * ratio).toInt().coerceIn(0, height - 1)
                            val right = (appBounds.right * ratio).toInt().coerceIn(left + 1, width)
                            val bottom = (appBounds.bottom * ratio).toInt().coerceIn(top + 1, height)
                            val pixels = IntArray((right - left) * (bottom - top))
                            scaled!!.getPixels(pixels, 0, right - left, left, top, right - left, bottom - top)
                            val pixelBytes = ByteBuffer.allocate(pixels.size * 4).apply { asIntBuffer().put(pixels) }.array()
                            val digest = MessageDigest.getInstance("SHA-256").apply { update("$left,$top,$right,$bottom,$width,$height".toByteArray()) }.digest(pixelBytes)
                            val fingerprint = Base64.encodeToString(digest, Base64.NO_WRAP)
                            val encoded = Base64.encodeToString(bytes, Base64.NO_WRAP)
                            main.post {
                                main.removeCallbacks(timeout)
                                if (!valid()) finish("The active screen changed during capture. Observe again.", changed = true)
                                else if (!settled.get()) {
                                    observation.put("image", JSObject().apply {
                                        put("base64", encoded); put("mimeType", "image/jpeg"); put("width", width); put("height", height)
                                        put("displayWidth", displayWidth); put("displayHeight", displayHeight); put("coordinateSystem", "screenshot_pixels")
                                    })
                                    finish(null, PhoneCapture(observation, fingerprint, width, height, displayWidth, displayHeight))
                                }
                            }
                        } catch (error: Exception) { finish(error.message ?: "Screenshot encoding failed.") }
                        finally { if (scaled !== bitmap) scaled?.recycle(); bitmap.recycle() }
                    } } catch (error: Exception) { bitmap.recycle(); finish("Screenshot service is closed.") }
                }
            })
        } catch (error: Exception) { main.removeCallbacks(timeout); finish(error.message ?: "Screenshot permission is unavailable.") }
        }, maxOf(0L, 360 - (android.os.SystemClock.elapsedRealtime() - lastCaptureAt)))
    }
    fun cancelPending() { cancel?.invoke() }
    fun close() { cancelPending(); encoder.shutdownNow() }
}
