package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.animation.ValueAnimator
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.view.animation.DecelerateInterpolator
import android.widget.ImageButton
import android.widget.LinearLayout
import kotlin.math.abs

/** A small, optional accessibility overlay; the existing task service owns Stop. */
class PhoneAssistantOverlay(private val service: AccessibilityService) {
    private val manager = service.getSystemService(WindowManager::class.java)
    private var view: LinearLayout? = null
    private var position: WindowManager.LayoutParams? = null
    private var snap: ValueAnimator? = null
    private val preferences = service.getSharedPreferences("phone-assistant", android.content.Context.MODE_PRIVATE)
    private val density = service.resources.displayMetrics.density
    private fun dp(value: Int) = (value * density).toInt()
    private fun maxX() = maxOf(dp(8), service.resources.displayMetrics.widthPixels - dp(58))
    private fun maxY() = maxOf(dp(24), service.resources.displayMetrics.heightPixels - dp(160))
    fun reposition() {
        val params = position ?: return
        val layout = view ?: return
        snap?.cancel()
        params.x = if (preferences.getBoolean("leftEdge", false)) dp(8) else maxX()
        params.y = (preferences.getFloat("verticalPosition", 0.22f) * maxY()).toInt().coerceIn(dp(24), maxY())
        manager.updateViewLayout(layout, params)
    }
    private fun dock(layout: LinearLayout, params: WindowManager.LayoutParams) {
        val left = params.x < maxX() / 2
        val destination = if (left) dp(8) else maxX()
        preferences.edit().putBoolean("leftEdge", left).putFloat("verticalPosition", params.y.toFloat() / maxY()).apply()
        snap?.cancel()
        if (!ValueAnimator.areAnimatorsEnabled()) { params.x = destination; manager.updateViewLayout(layout, params); return }
        snap = ValueAnimator.ofInt(params.x, destination).apply {
            duration = 220; interpolator = DecelerateInterpolator()
            addUpdateListener { params.x = it.animatedValue as Int; if (view === layout && layout.isAttachedToWindow) manager.updateViewLayout(layout, params) }
            start()
        }
    }
    fun show() {
        if (view != null) return
        val layout = LinearLayout(service).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply { setColor(service.getColor(R.color.nexa_background)); cornerRadius = dp(26).toFloat(); setStroke(dp(1), service.getColor(R.color.nexa_overlay_border)) }
            elevation = dp(6).toFloat()
            setPadding(dp(3), dp(3), dp(3), dp(3))
        }
        val params = WindowManager.LayoutParams(dp(50), WindowManager.LayoutParams.WRAP_CONTENT,
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
                MotionEvent.ACTION_DOWN -> { snap?.cancel(); layout.animate().cancel(); layout.translationY = 0f; downX = event.rawX; downY = event.rawY; startX = params.x; startY = params.y; dragged = false; false }
                MotionEvent.ACTION_MOVE -> {
                    if (abs(event.rawX-downX) + abs(event.rawY-downY) > dp(8)) dragged = true
                    if (dragged) {
                        control.cancelLongPress(); control.isPressed = false
                        params.x = (startX+(event.rawX-downX).toInt()).coerceIn(dp(8), maxX())
                        params.y = (startY+(event.rawY-downY).toInt()).coerceIn(dp(24), maxY())
                        manager.updateViewLayout(layout, params)
                    }; dragged
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> { if (dragged) { control.isPressed = false; dock(layout, params) }; dragged }
                else -> false
            }
        }
        layout.addView(open); layout.addView(stop); manager.addView(layout, params); view = layout; position = params
        // One gentle arrival motion, without an idle animation loop or service.
        if (ValueAnimator.areAnimatorsEnabled()) layout.animate().translationY(-dp(3).toFloat()).setDuration(350).withEndAction {
            if (view === layout) layout.animate().translationY(0f).setDuration(450).start()
        }.start()
    }
    fun hide() { snap?.cancel(); snap = null; view?.let { it.animate().cancel(); manager.removeView(it) }; view = null; position = null }
    fun suspend() { snap?.cancel(); view?.let { it.animate().cancel(); it.translationY = 0f; it.visibility = View.GONE } }
    fun restore() { view?.visibility = View.VISIBLE }
}
