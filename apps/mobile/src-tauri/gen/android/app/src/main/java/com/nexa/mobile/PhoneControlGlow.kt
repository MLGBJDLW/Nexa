package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.animation.ValueAnimator
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.view.View
import android.view.WindowManager

/** Non-intercepting feedback. Hidden while evidence pixels are captured. */
class PhoneControlGlow(private val service: AccessibilityService) {
    private val manager = service.getSystemService(WindowManager::class.java)
    private var view: View? = null
    private var pulse: ValueAnimator? = null
    private var active = false
    fun setActive(value: Boolean) {
        active = value
        if (!value) { close(); return }
        if (view == null) {
            val density = service.resources.displayMetrics.density
            val border = View(service).apply {
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                background = GradientDrawable().apply { setColor(android.graphics.Color.TRANSPARENT); cornerRadius=28*density; setStroke((4*density).toInt(), android.graphics.Color.rgb(126,102,246)) }
            }
            val params = WindowManager.LayoutParams(-1,-1,WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,PixelFormat.TRANSLUCENT)
            manager.addView(border,params); view=border
            if (ValueAnimator.areAnimatorsEnabled()) pulse=ValueAnimator.ofFloat(.45f,1f).apply { duration=1000; repeatCount=ValueAnimator.INFINITE; repeatMode=ValueAnimator.REVERSE; addUpdateListener { border.alpha=it.animatedValue as Float }; start() }
        }
    }
    fun suspend() { view?.visibility=View.GONE; pulse?.pause() }
    fun restore() { if (active) { view?.visibility=View.VISIBLE; pulse?.resume() } }
    fun close() { pulse?.cancel(); pulse=null; view?.let { manager.removeView(it) }; view=null }
}
