package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.animation.ValueAnimator
import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.os.PowerManager
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.GestureDetector
import android.widget.Toast
import android.view.WindowManager
import android.view.animation.LinearInterpolator
import android.view.animation.AccelerateDecelerateInterpolator
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import kotlin.math.abs
import kotlin.math.sin
import kotlin.math.PI

/** A small, optional accessibility overlay; the existing task service owns Stop. */
class PhoneAssistantOverlay(private val service: AccessibilityService) {
    companion object {
        // Keep screen text in process memory, outside Binder's intent size limit.
        private var pendingTranslation: String? = null
        fun consumeTranslation(): String? = pendingTranslation.also { pendingTranslation = null }
    }
    private val manager = service.getSystemService(WindowManager::class.java)
    private var view: FrameLayout? = null
    private var surface: LinearLayout? = null
    private var hover: ValueAnimator? = null
    private var suspended = false
    private var quickActionHidden = false
    private var receiverRegistered = false
    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == Intent.ACTION_SCREEN_OFF) { snap?.cancel(); stopHover() } else startHover()
        }
    }
    private var position: WindowManager.LayoutParams? = null
    private var snap: ValueAnimator? = null
    private var quickActions: LinearLayout? = null
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    private val preferences = service.getSharedPreferences("phone-assistant", android.content.Context.MODE_PRIVATE)
    private val density = service.resources.displayMetrics.density
    private fun dp(value: Int) = (value * density).toInt()
    private fun maxX() = maxOf(dp(8), service.resources.displayMetrics.widthPixels - dp(74))
    private fun maxY() = maxOf(dp(24), service.resources.displayMetrics.heightPixels - dp(160))
    fun reposition() {
        quickActions?.visibility = View.GONE
        val params = position ?: return
        val layout = view ?: return
        snap?.cancel()
        params.x = if (preferences.getBoolean("leftEdge", false)) dp(8) else maxX()
        params.y = (preferences.getFloat("verticalPosition", 0.22f) * maxY()).toInt().coerceIn(dp(24), maxY())
        manager.updateViewLayout(layout, params)
    }
    private fun stopHover() {
        hover?.cancel(); hover = null
        surface?.let { it.translationY = 0f; it.scaleX = 1f; it.scaleY = 1f; it.elevation = dp(8).toFloat() }
    }
    private fun startHover() {
        stopHover()
        val panel = surface ?: return
        val layout = view ?: return
        if (suspended || quickActionHidden || !layout.isAttachedToWindow || !ValueAnimator.areAnimatorsEnabled()
            || !service.getSystemService(PowerManager::class.java).isInteractive) return
        hover = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 2600; repeatCount = ValueAnimator.INFINITE; repeatMode = ValueAnimator.REVERSE
            interpolator = AccelerateDecelerateInterpolator()
            addUpdateListener {
                val phase = it.animatedValue as Float
                // Only compositor properties move at rest: no WindowManager
                // relayout, polling, wake lock, or independent background service.
                panel.translationY = -dp(4) * phase
                panel.elevation = dp(8) + dp(4) * phase
                panel.scaleX = 1f + .018f * phase; panel.scaleY = panel.scaleX
            }
            start()
        }
    }
    private fun dock(layout: FrameLayout, params: WindowManager.LayoutParams) {
        val minimum = dp(8)
        val motion = OverlayDockMotion(params.x, minimum, maxX(), density)
        val destination = motion.destination
        val left = destination == minimum
        preferences.edit().putBoolean("leftEdge", left).putFloat("verticalPosition", params.y.toFloat() / maxY()).apply()
        snap?.cancel(); stopHover()
        if (!ValueAnimator.areAnimatorsEnabled()) { params.x = destination; manager.updateViewLayout(layout, params); return }
        snap = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = motion.durationMs; interpolator = LinearInterpolator()
            addUpdateListener {
                val progress = it.animatedValue as Float
                params.x = motion.x(progress)
                if (view === layout && layout.isAttachedToWindow) manager.updateViewLayout(layout, params)
            }
            addListener(object : AnimatorListenerAdapter() {
                private var cancelled = false
                override fun onAnimationCancel(animation: Animator) { cancelled = true }
                override fun onAnimationEnd(animation: Animator) { if (!cancelled && view === layout) startHover() }
            })
            start()
        }
    }
    fun show() {
        if (view != null) return
        val panel = LinearLayout(service).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply { setColor(service.getColor(R.color.nexa_background)); cornerRadius = dp(26).toFloat(); setStroke(dp(1), service.getColor(R.color.nexa_overlay_border)) }
            elevation = dp(8).toFloat()
            setPadding(dp(3), dp(3), dp(3), dp(3))
        }
        // Transparent inset leaves room for shadows and the suspended motion.
        val layout = FrameLayout(service).apply {
            setPadding(dp(8), dp(10), dp(8), dp(10)); clipChildren = false; clipToPadding = false
            addView(panel, FrameLayout.LayoutParams(dp(50), FrameLayout.LayoutParams.WRAP_CONTENT))
        }
        val params = WindowManager.LayoutParams(dp(66), WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT).apply {
                gravity = Gravity.TOP or Gravity.LEFT
                x = if (preferences.getBoolean("leftEdge", false)) dp(8) else maxX()
                y = (preferences.getFloat("verticalPosition", 0.22f) * maxY()).toInt().coerceIn(dp(24), maxY())
            }
        fun button(icon: Int, description: String) = ImageButton(service).apply {
            setImageResource(icon); imageTintList = ColorStateList.valueOf(service.getColor(R.color.nexa_overlay_text))
            setBackgroundColor(android.graphics.Color.TRANSPARENT); contentDescription = description
            layoutParams = LinearLayout.LayoutParams(dp(44), dp(44))
        }
        val open = button(R.drawable.ic_nexa_notification, service.getString(R.string.main_activity_title))
        val stop = button(android.R.drawable.ic_media_pause, service.getString(R.string.task_stop)).apply {
            visibility = View.GONE
            setOnClickListener { service.startService(Intent(service, TaskService::class.java).setAction("stop")); visibility = View.GONE }
        }
        open.setOnClickListener { service.startActivity(Intent(service, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)) }
        val actions = LinearLayout(service).apply { orientation = LinearLayout.VERTICAL; visibility = View.GONE }
        quickActions = actions
        fun screenshot() {
            try { (service as PhoneAccessibilityService).ensureOverlayIdle() }
            catch (error: Exception) { Toast.makeText(service, error.message, Toast.LENGTH_LONG).show(); return }
            actions.visibility = View.GONE
            setQuickActionHidden(true)
            main.postDelayed({
                if (view !== layout) return@postDelayed
                try {
                    (service as PhoneAccessibilityService).ensureOverlayIdle()
                    if (android.os.Build.VERSION.SDK_INT < 28 || !service.performGlobalAction(AccessibilityService.GLOBAL_ACTION_TAKE_SCREENSHOT)) {
                        Toast.makeText(service, R.string.overlay_screenshot_unavailable, Toast.LENGTH_LONG).show()
                    }
                } catch (error: Exception) {
                    Toast.makeText(service, error.message ?: service.getString(R.string.overlay_screenshot_unavailable), Toast.LENGTH_LONG).show()
                } finally { main.postDelayed({ if (view === layout) setQuickActionHidden(false) }, 400) }
            }, 160)
        }
        actions.addView(button(android.R.drawable.ic_menu_camera, service.getString(R.string.overlay_screenshot)).apply { setOnClickListener { screenshot() } })
        actions.addView(button(android.R.drawable.ic_menu_sort_alphabetically, service.getString(R.string.overlay_translate)).apply {
            setOnClickListener {
                try {
                    val text = (service as PhoneAccessibilityService).overlayTranslationText()
                    actions.visibility = View.GONE
                    pendingTranslation = service.getString(R.string.overlay_translate_prompt, text)
                    service.startActivity(Intent(service, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                        .putExtra("nexa.overlayDraft", true))
                } catch (error: Exception) { Toast.makeText(service, error.message ?: service.getString(R.string.overlay_text_unavailable), Toast.LENGTH_LONG).show() }
            }
        })
        stop.visibility = View.VISIBLE
        actions.addView(stop)
        open.setOnLongClickListener {
            actions.visibility = if (actions.visibility == View.VISIBLE) View.GONE else View.VISIBLE
            if (actions.visibility == View.VISIBLE) {
                stop.visibility = View.VISIBLE
                params.y = params.y.coerceAtMost(maxOf(dp(24), service.resources.displayMetrics.heightPixels - dp(240)))
                manager.updateViewLayout(layout, params)
            }
            true
        }
        open.contentDescription = service.getString(R.string.overlay_gestures)
        val gestures = GestureDetector(service, object : GestureDetector.SimpleOnGestureListener() {
            override fun onDown(event: MotionEvent) = true
            override fun onSingleTapConfirmed(event: MotionEvent): Boolean { open.performClick(); return true }
            override fun onDoubleTap(event: MotionEvent): Boolean { screenshot(); return true }
            override fun onLongPress(event: MotionEvent) { open.performLongClick() }
        })
        var downX = 0f; var downY = 0f; var startX = 0; var startY = 0; var dragged = false
        open.setOnTouchListener { control, event ->
            if (!dragged || event.actionMasked == MotionEvent.ACTION_DOWN) gestures.onTouchEvent(event)
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> { snap?.cancel(); stopHover(); downX = event.rawX; downY = event.rawY; startX = params.x; startY = params.y; dragged = false; true }
                MotionEvent.ACTION_MOVE -> {
                    if (!dragged && abs(event.rawX-downX) + abs(event.rawY-downY) > dp(8)) {
                        dragged = true; actions.visibility = View.GONE
                        MotionEvent.obtain(event).also { it.action = MotionEvent.ACTION_CANCEL; gestures.onTouchEvent(it); it.recycle() }
                    }
                    if (dragged) {
                        control.cancelLongPress(); control.isPressed = false
                        params.x = (startX+(event.rawX-downX).toInt()).coerceIn(dp(8), maxX())
                        params.y = (startY+(event.rawY-downY).toInt()).coerceIn(dp(24), maxY())
                        manager.updateViewLayout(layout, params)
                    }; true
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> { if (dragged) { control.isPressed = false; dock(layout, params) } else startHover(); true }
                else -> true
            }
        }
        panel.addView(open); panel.addView(actions); manager.addView(layout, params)
        view = layout; surface = panel; position = params; suspended = false
        val filter = IntentFilter().apply { addAction(Intent.ACTION_SCREEN_OFF); addAction(Intent.ACTION_SCREEN_ON) }
        if (android.os.Build.VERSION.SDK_INT >= 33) service.registerReceiver(screenReceiver, filter, Context.RECEIVER_NOT_EXPORTED)
        else @Suppress("DEPRECATION") service.registerReceiver(screenReceiver, filter)
        receiverRegistered = true
        // WindowManager attaches its root on the next traversal. Starting now
        // would skip the idle animation on the first appearance.
        layout.post { if (view === layout) startHover() }
    }
    fun hide() {
        main.removeCallbacksAndMessages(null); quickActions = null; quickActionHidden = false
        snap?.cancel(); snap = null; stopHover()
        if (receiverRegistered) { service.unregisterReceiver(screenReceiver); receiverRegistered = false }
        view?.let { manager.removeView(it) }; view = null; surface = null; position = null
    }
    private fun updateVisibility() {
        val hidden = suspended || quickActionHidden
        view?.visibility = if (hidden) View.GONE else View.VISIBLE
        if (hidden) stopHover() else startHover()
    }
    private fun setQuickActionHidden(hidden: Boolean) { quickActionHidden = hidden; snap?.cancel(); updateVisibility() }
    fun suspend() { suspended = true; snap?.cancel(); updateVisibility() }
    fun restore() { suspended = false; updateVisibility() }
}

/** Bounded edge rebound, independent of frame rate and Android UI state. */
internal class OverlayDockMotion(private val origin: Int, private val minimum: Int, private val maximum: Int, density: Float) {
    private val middle = (minimum + maximum) / 2f
    private val center = (1f - abs(origin - middle) / maxOf(1f, middle - minimum)).coerceIn(0f, 1f)
    val destination = if (origin < middle) minimum else maximum
    val durationMs = (650 - 220 * center).toLong()
    val amplitude = minOf(abs(origin - destination).toFloat(), (6 + 32 * center) * density)
    fun x(progress: Float): Int {
        val t = progress.coerceIn(0f, 1f)
        val travel = (t / .42f).coerceAtMost(1f)
        val bounce = ((t - .42f) / .58f).coerceAtLeast(0f)
        val displacement = if (t < .42f) (origin - destination) * (1f - travel) * (1f - travel)
            else (if (destination == minimum) 1 else -1) * amplitude * abs(sin(bounce * PI * 2)).toFloat() * (1f - bounce) * (1f - bounce)
        return (destination + displacement).toInt().coerceIn(minimum, maximum)
    }
}
