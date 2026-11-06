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
    private val manager = service.getSystemService(WindowManager::class.java)
    private var view: FrameLayout? = null
    private var surface: LinearLayout? = null
    private var hover: ValueAnimator? = null
    private var suspended = false
    private var receiverRegistered = false
    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == Intent.ACTION_SCREEN_OFF) { snap?.cancel(); stopHover() } else startHover()
        }
    }
    private var position: WindowManager.LayoutParams? = null
    private var snap: ValueAnimator? = null
    private val preferences = service.getSharedPreferences("phone-assistant", android.content.Context.MODE_PRIVATE)
    private val density = service.resources.displayMetrics.density
    private fun dp(value: Int) = (value * density).toInt()
    private fun maxX() = maxOf(dp(8), service.resources.displayMetrics.widthPixels - dp(74))
    private fun maxY() = maxOf(dp(24), service.resources.displayMetrics.heightPixels - dp(160))
    fun reposition() {
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
        if (suspended || !layout.isAttachedToWindow || !ValueAnimator.areAnimatorsEnabled()
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
        open.setOnLongClickListener { stop.visibility = if (stop.visibility == View.VISIBLE) View.GONE else View.VISIBLE; true }
        var downX = 0f; var downY = 0f; var startX = 0; var startY = 0; var dragged = false
        open.setOnTouchListener { control, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> { snap?.cancel(); stopHover(); downX = event.rawX; downY = event.rawY; startX = params.x; startY = params.y; dragged = false; false }
                MotionEvent.ACTION_MOVE -> {
                    if (abs(event.rawX-downX) + abs(event.rawY-downY) > dp(8)) dragged = true
                    if (dragged) {
                        control.cancelLongPress(); control.isPressed = false
                        params.x = (startX+(event.rawX-downX).toInt()).coerceIn(dp(8), maxX())
                        params.y = (startY+(event.rawY-downY).toInt()).coerceIn(dp(24), maxY())
                        manager.updateViewLayout(layout, params)
                    }; dragged
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> { if (dragged) { control.isPressed = false; dock(layout, params) } else startHover(); dragged }
                else -> false
            }
        }
        panel.addView(open); panel.addView(stop); manager.addView(layout, params)
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
        snap?.cancel(); snap = null; stopHover()
        if (receiverRegistered) { service.unregisterReceiver(screenReceiver); receiverRegistered = false }
        view?.let { manager.removeView(it) }; view = null; surface = null; position = null
    }
    fun suspend() { suspended = true; snap?.cancel(); stopHover(); view?.visibility = View.GONE }
    fun restore() { suspended = false; view?.visibility = View.VISIBLE; startHover() }
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
