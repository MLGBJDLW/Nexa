package com.nexa.mobile

import android.os.Bundle
import android.content.res.Configuration
import androidx.activity.enableEdgeToEdge
import androidx.activity.OnBackPressedCallback
import android.webkit.WebView
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsAnimationCompat

class MainActivity : TauriActivity() {
  private var notificationWebView: WebView? = null

  override fun onNewIntent(intent: android.content.Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    notificationWebView?.evaluateJavascript("window.dispatchEvent(new Event('nexa:notificationOpen'))", null)
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    enableEdgeToEdge()
    window.setBackgroundDrawableResource(R.color.nexa_background)
  }
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    notificationWebView = webView
    webView.post {
      val content = findViewById<View>(android.R.id.content)
      val left = content.paddingLeft; val top = content.paddingTop
      val right = content.paddingRight; val bottom = content.paddingBottom
      var animating = false
      fun applyKeyboard(insets: WindowInsetsCompat) {
        val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
        val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
        val obstruction = maxOf(keyboard, bars.bottom)
        if (content.paddingLeft != left + bars.left || content.paddingTop != top + bars.top ||
            content.paddingRight != right + bars.right || content.paddingBottom != bottom + obstruction) {
          content.setPadding(left + bars.left, top + bars.top, right + bars.right, bottom + obstruction)
        }
      }
      // Edge-to-edge WebViews do not resize their visual viewport for IME
      // overlays. Resize the native content owner and follow the system motion,
      // keeping content clear of system bars and cutouts in either state.
      // Consume these insets so WebView CSS does not apply them a second time.
      ViewCompat.setOnApplyWindowInsetsListener(content) { _, insets ->
        if (!animating) applyKeyboard(insets)
        WindowInsetsCompat.CONSUMED
      }
      ViewCompat.setWindowInsetsAnimationCallback(content, object : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
        override fun onPrepare(animation: WindowInsetsAnimationCompat) {
          if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) animating = true
        }
        override fun onProgress(insets: WindowInsetsCompat, running: MutableList<WindowInsetsAnimationCompat>): WindowInsetsCompat {
          applyKeyboard(insets); return WindowInsetsCompat.CONSUMED
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
