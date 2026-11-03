package com.nexa.mobile

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.activity.OnBackPressedCallback
import android.webkit.WebView
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsAnimationCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.post {
      val content = findViewById<View>(android.R.id.content)
      val left = content.paddingLeft; val top = content.paddingTop
      val right = content.paddingRight; val bottom = content.paddingBottom
      var animating = false
      fun applyKeyboard(insets: WindowInsetsCompat) {
        val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
        if (content.paddingBottom != bottom + keyboard) content.setPadding(left, top, right, bottom + keyboard)
      }
      // Edge-to-edge WebViews do not resize their visual viewport for IME
      // overlays. Resize the native content owner and follow the system motion.
      ViewCompat.setOnApplyWindowInsetsListener(content) { _, insets ->
        if (!animating) applyKeyboard(insets)
        insets
      }
      ViewCompat.setWindowInsetsAnimationCallback(content, object : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
        override fun onPrepare(animation: WindowInsetsAnimationCompat) {
          if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) animating = true
        }
        override fun onProgress(insets: WindowInsetsCompat, running: MutableList<WindowInsetsAnimationCompat>): WindowInsetsCompat {
          applyKeyboard(insets); return insets
        }
        override fun onEnd(animation: WindowInsetsAnimationCompat) {
          if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) { animating = false; ViewCompat.requestApplyInsets(content) }
        }
      })
      ViewCompat.requestApplyInsets(content)
    }
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("""(() => { const dialogs = document.querySelectorAll('dialog[open]'); const top = dialogs[dialogs.length - 1]; if (!top) return false; top.dispatchEvent(new Event('cancel', { cancelable: true })); return true; })()""") { handled ->
          if (handled != "true") { isEnabled = false; onBackPressedDispatcher.onBackPressed(); isEnabled = true }
        }
      }
    })
  }
}
