package com.nexa.mobile

import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import android.widget.*
import app.tauri.plugin.JSObject
import org.json.JSONObject
import java.util.UUID

/** Image-first screen assistance, inside the existing accessibility service. */
class PhoneScreenPanel(private val service: PhoneAccessibilityService) {
    private val manager = service.getSystemService(WindowManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var root: LinearLayout? = null
    private var image: ImageView? = null
    private var canvas: FrameLayout? = null
    private var status: TextView? = null
    private var input: EditText? = null
    private var params: WindowManager.LayoutParams? = null
    private var frame: JSObject? = null
    private var request: String? = null
    private var live = false
    private var capturing = false
    private var queuedCapture: Pair<Boolean, String?>? = null
    private var generation = 0
    private var translated = false
    fun isActive(id: String) = request == id && root != null
    private val density get() = service.resources.displayMetrics.density
    private fun dp(value: Int) = (value * density).toInt()
    private fun button(label: Int, action: () -> Unit) = Button(service).apply { text = service.getString(label); isAllCaps = false; setOnClickListener { action() } }

    fun open(translate: Boolean) {
        close(); generation++
        val panel = LinearLayout(service).apply {
            orientation = LinearLayout.VERTICAL; setPadding(dp(12), dp(10), dp(12), dp(10))
            background = GradientDrawable().apply { setColor(service.getColor(R.color.nexa_background)); cornerRadius = dp(22).toFloat(); setStroke(dp(1), service.getColor(R.color.nexa_overlay_border)) }
            elevation = dp(12).toFloat()
        }
        val controls = LinearLayout(service).apply { gravity = Gravity.CENTER_VERTICAL }
        controls.addView(button(R.string.overlay_close) { close() })
        controls.addView(button(R.string.overlay_live) { live = !live; if (live) { translated = false; capture(false) }; status?.text = service.getString(if (live) R.string.overlay_live_active else R.string.overlay_paused) })
        controls.addView(button(R.string.overlay_translate) { capture(true) })
        panel.addView(controls)
        canvas = FrameLayout(service).apply {
            image = ImageView(service).apply { scaleType = ImageView.ScaleType.FIT_CENTER; contentDescription = service.getString(R.string.overlay_screenshot) }
            addView(image, FrameLayout.LayoutParams(-1, -1))
        }
        panel.addView(canvas, LinearLayout.LayoutParams(-1, dp(155)))
        status = TextView(service).apply { setTextColor(service.getColor(R.color.nexa_overlay_text)); textSize = 12f; setPadding(0, dp(6), 0, dp(6)); text = service.getString(R.string.overlay_screen_hint); maxHeight = dp(100); isVerticalScrollBarEnabled = true; movementMethod = android.text.method.ScrollingMovementMethod() }
        panel.addView(status)
        val editor = LinearLayout(service).apply { gravity = Gravity.CENTER_VERTICAL }
        input = EditText(service).apply {
            hint = service.getString(R.string.overlay_input); setTextColor(service.getColor(R.color.nexa_overlay_text)); setHintTextColor(service.getColor(R.color.nexa_overlay_text))
            maxLines = 3; textSize = 14f
            setOnTouchListener { _, event ->
                if (event.action == android.view.MotionEvent.ACTION_DOWN) params?.let { position ->
                    position.flags = position.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE.inv()
                    root?.let { manager.updateViewLayout(it, position) }
                }
                false
            }
        }
        editor.addView(input, LinearLayout.LayoutParams(0, -2, 1f))
        editor.addView(button(R.string.overlay_send) {
            if (!input?.text.isNullOrBlank()) {
                service.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(input?.windowToken, 0)
                input?.clearFocus()
                params?.let { position -> position.flags = position.flags or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE; root?.let { manager.updateViewLayout(it, position) } }
                capture(false, input?.text.toString())
            }
        })
        panel.addView(editor)
        val position = WindowManager.LayoutParams(minOf(service.resources.displayMetrics.widthPixels-dp(16), dp(520)), -2,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT).apply { gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL; y = dp(24); softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE }
        manager.addView(panel, position); root = panel; params = position
        capture(translate)
    }

    private fun capture(translate: Boolean, prompt: String? = null) {
        if (root == null || request != null) return
        if (capturing) { queuedCapture = translate to prompt; return }
        capturing = true
        val mine = generation
        service.overlayCapture { result ->
            capturing = false
            val queued = queuedCapture; queuedCapture = null
            if (generation != mine || root == null) {
                if (root != null && queued != null) capture(queued.first, queued.second)
                return@overlayCapture
            }
            result.fold({ captured ->
                frame = JSObject(captured.state.getJSONObject("image").toString())
                val bytes = Base64.decode(frame!!.getString("base64"), Base64.NO_WRAP)
                image?.setImageBitmap(BitmapFactory.decodeByteArray(bytes, 0, bytes.size))
                canvas?.let { while (it.childCount > 1) it.removeViewAt(it.childCount-1) }
                if (translate || prompt != null) submit(translate, prompt ?: "")
                else if (live) main.postDelayed({ if (mine == generation && live) capture(false) }, 1200)
            }, { error -> status?.text = error.message; live = false })
            if (queued != null && request == null) capture(queued.first, queued.second)
        }
    }

    private fun submit(translate: Boolean, prompt: String) {
        val plugin = PhonePlugin.instance
        if (plugin == null) { status?.setText(R.string.overlay_open_first); return }
        val id = UUID.randomUUID().toString(); request = id; translated = translate; live = false
        status?.setText(R.string.overlay_working)
        plugin.overlayRequest(JSObject().apply { put("requestId", id); put("image", frame); put("translate", translate); put("prompt", prompt); put("language", service.resources.configuration.locales[0].toLanguageTag()) })
    }

    fun result(value: JSObject) {
        if (value.optString("requestId") != request) return
        request = null
        if (value.has("error")) { status?.text = value.optString("error"); return }
        val text = value.optString("text")
        if (!translated) { status?.text = text; return }
        try {
            val clean = text.trim().removePrefix("```json").removePrefix("```").removeSuffix("```").trim()
            val regions = JSONObject(clean).getJSONArray("regions")
            check(regions.length() > 0) { service.getString(R.string.overlay_text_unavailable) }
            // A frozen image canvas makes the translation reversible and keeps
            // model-proposed bounds away from live app controls.
            canvas?.layoutParams = LinearLayout.LayoutParams(-1, (service.resources.displayMetrics.heightPixels * .60).toInt())
            status?.setText(R.string.overlay_translation_hint)
            val mine = generation
            canvas?.post {
                if (mine != generation) return@post
                val area = canvas ?: return@post
                val source = frame ?: return@post
                val ratio = minOf(area.width.toFloat()/source.getInt("width"), area.height.toFloat()/source.getInt("height"))
                val width = source.getInt("width")*ratio; val height = source.getInt("height")*ratio
                val left = (area.width-width)/2; val top = (area.height-height)/2
                for (index in 0 until minOf(256, regions.length())) {
                    val region = regions.optJSONObject(index) ?: continue
                    val x=region.optDouble("x"); val y=region.optDouble("y"); val w=region.optDouble("width"); val h=region.optDouble("height")
                    if (!x.isFinite() || !y.isFinite() || !w.isFinite() || !h.isFinite() || x !in 0.0..1.0 || y !in 0.0..1.0 || w <= 0 || h <= 0) continue
                    val translatedText = TextView(service).apply {
                        this.text = region.optString("text"); setTextColor(Color.rgb(25,25,30)); setBackgroundColor(Color.rgb(248,248,250)); textSize = 12f
                        setAutoSizeTextTypeUniformWithConfiguration(8,16,1,android.util.TypedValue.COMPLEX_UNIT_SP)
                        setPadding(dp(2), 0, dp(2), 0); isClickable = true
                        setOnClickListener { visibility = View.INVISIBLE; postDelayed({ visibility = View.VISIBLE }, 1500) }
                    }
                    area.addView(translatedText, FrameLayout.LayoutParams(maxOf(dp(12),(minOf(w,1-x)*width).toInt()), maxOf(dp(16),(minOf(h,1-y)*height).toInt())).apply { leftMargin=(left+x*width).toInt(); topMargin=(top+y*height).toInt() })
                }
            }
        } catch (_: Exception) { status?.setText(R.string.overlay_translation_failed) }
    }

    fun suspend() { root?.visibility = View.GONE }
    fun restore() { root?.visibility = View.VISIBLE }
    fun close() {
        generation++; main.removeCallbacksAndMessages(null); live = false; queuedCapture = null
        request?.let { PhonePlugin.instance?.overlayRequest(JSObject().apply { put("requestId", it); put("cancel", true) }) }; request = null
        root?.let { service.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(it.windowToken, 0); manager.removeView(it) }
        root = null; params = null; image?.setImageDrawable(null); image = null; canvas = null; frame = null; input = null; status = null
    }
}
