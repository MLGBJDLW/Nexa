package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.ImageButton
import android.widget.LinearLayout
import kotlin.math.abs

/** A small, optional accessibility overlay; the existing task service owns Stop. */
class PhoneAssistantOverlay(private val service: AccessibilityService) {
    private val manager = service.getSystemService(WindowManager::class.java)
    private var view: LinearLayout? = null
    private val density = service.resources.displayMetrics.density
    private fun dp(value: Int) = (value * density).toInt()
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
            PixelFormat.TRANSLUCENT).apply { gravity = Gravity.TOP or Gravity.END; x = dp(10); y = dp(180) }
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
        open.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> { downX = event.rawX; downY = event.rawY; startX = params.x; startY = params.y; dragged = false; false }
                MotionEvent.ACTION_MOVE -> {
                    if (abs(event.rawX-downX) + abs(event.rawY-downY) > dp(8)) dragged = true
                    if (dragged) {
                        params.x = (startX-(event.rawX-downX).toInt()).coerceIn(0, maxOf(0, service.resources.displayMetrics.widthPixels-dp(50)))
                        params.y = (startY+(event.rawY-downY).toInt()).coerceIn(dp(24), maxOf(dp(24), service.resources.displayMetrics.heightPixels-dp(130)))
                        manager.updateViewLayout(layout, params)
                    }; dragged
                }
                MotionEvent.ACTION_UP -> dragged
                else -> false
            }
        }
        layout.addView(open); layout.addView(stop); manager.addView(layout, params); view = layout
    }
    fun hide() { view?.let { manager.removeView(it) }; view = null }
    fun suspend() { view?.visibility = View.GONE }
    fun restore() { view?.visibility = View.VISIBLE }
}
