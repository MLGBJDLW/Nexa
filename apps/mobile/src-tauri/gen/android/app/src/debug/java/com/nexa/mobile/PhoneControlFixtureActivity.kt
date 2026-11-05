package com.nexa.mobile

import android.app.Activity
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/** Debug-only device acceptance surface; no accounts, network or real messages. */
class PhoneControlFixtureActivity : Activity() {
    companion object { @Volatile var current: PhoneControlFixtureActivity? = null }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        current = this
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(30, 60, 30, 30); isFocusableInTouchMode = true }
        layout.addView(TextView(this).apply { text = "Phone control acceptance"; textSize = 22f })
        layout.addView(EditText(this).apply { hint = "Message"; contentDescription = "Semantic message"; id = 1001 })
        layout.addView(object : EditText(this) {
            override fun performAccessibilityAction(action: Int, arguments: Bundle?): Boolean {
                if (action == AccessibilityNodeInfo.ACTION_SET_TEXT) return false
                return super.performAccessibilityAction(action, arguments)
            }
        }.apply { hint = "Native editor"; contentDescription = "Native message"; id = 1002 })
        val result = TextView(this).apply { text = "Applied 0"; id = 1003 }
        var count = 0
        layout.addView(Button(this).apply { text = "Apply message"; contentDescription = "Apply message"; setOnClickListener { result.text = "Applied ${++count}" } })
        layout.addView(result)
        setContentView(layout)
        layout.requestFocus()
    }
    override fun onDestroy() { if (current === this) current = null; super.onDestroy() }
}
