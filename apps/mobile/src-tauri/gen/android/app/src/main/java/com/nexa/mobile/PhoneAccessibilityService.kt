package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.graphics.Rect
import android.os.Bundle
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

class PhoneAccessibilityService : AccessibilityService() {
    companion object { @Volatile var instance: PhoneAccessibilityService? = null }
    private data class Target(val node: AccessibilityNodeInfo, val signature: String)
    private val nodes = mutableMapOf<Int, Target>()
    private var observation = ""
    private var observedPackage = ""
    private var observedAt = 0L
    override fun onServiceConnected() { instance = this }
    override fun onAccessibilityEvent(event: AccessibilityEvent?) { /* Read only on explicit tool observation. */ }
    override fun onInterrupt() { invalidate() }
    override fun onDestroy() { invalidate(); instance = null; super.onDestroy() }
    @Suppress("DEPRECATION") private fun invalidate() { nodes.values.forEach { it.node.recycle() }; nodes.clear(); observation = "" }
    private fun signature(node: AccessibilityNodeInfo): String {
        val bounds = Rect(); node.getBoundsInScreen(bounds)
        return "${node.windowId}|${node.packageName}|${node.viewIdResourceName}|${node.className}|${node.text}|${node.contentDescription}|${bounds.flattenToString()}|${node.isEnabled}|${node.isPassword}"
    }
    @Synchronized fun observe(): JSObject {
        invalidate()
        val root = rootInActiveWindow ?: error("The screen is locked or its UI is unavailable.")
        observedPackage = root.packageName?.toString() ?: error("The active app cannot be identified.")
        observation = UUID.randomUUID().toString(); observedAt = android.os.SystemClock.elapsedRealtime()
        val result = JSONArray(); var visited = 0; var truncated = false
        fun walk(node: AccessibilityNodeInfo, depth: Int) {
            if (visited++ >= 512 || depth > 40) { truncated = true; return }
            if (node.isVisibleToUser && !node.isPassword) {
                val id = nodes.size
                val bounds = Rect(); node.getBoundsInScreen(bounds)
                @Suppress("DEPRECATION") val copy = AccessibilityNodeInfo.obtain(node)
                nodes[id] = Target(copy, signature(node))
                result.put(JSONObject().put("nodeId",id).put("text",node.text?.toString()?.take(2000) ?: "")
                    .put("description",node.contentDescription?.toString()?.take(1000) ?: "").put("class",node.className)
                    .put("clickable",node.isClickable).put("editable",node.isEditable).put("scrollable",node.isScrollable)
                    .put("bounds",JSONArray(listOf(bounds.left,bounds.top,bounds.right,bounds.bottom))))
            }
            for (i in 0 until node.childCount) {
                if (visited >= 512) { truncated = true; break }
                val child = node.getChild(i) ?: continue
                walk(child,depth+1)
                @Suppress("DEPRECATION") child.recycle()
            }
        }
        walk(root,0)
        @Suppress("DEPRECATION") root.recycle()
        return JSObject().apply { put("observationId",observation); put("packageName",observedPackage); put("nodes",result); put("truncated",truncated); put("sensitiveFieldsOmitted",true) }
    }
    @Synchronized fun action(args: JSObject): JSObject {
        check(!TaskService.stopRequested.get()) { "The task was stopped." }
        val operation = args.getString("action")
        if (operation == "observe") return observe()
        require(args.getString("observationId") == observation && observation.isNotEmpty()) { "Observe the screen again before acting." }
        require(android.os.SystemClock.elapsedRealtime() - observedAt < 60_000) { "The observation expired; observe again." }
        val currentRoot = rootInActiveWindow ?: error("The active UI is unavailable.")
        val currentPackage = currentRoot.packageName?.toString()
        @Suppress("DEPRECATION") currentRoot.recycle()
        require(currentPackage == observedPackage) { "The foreground app changed; observe again." }
        if (operation != "open_app") require(args.getString("packageName") == observedPackage) { "The target package must match the observation." }
        val accepted = when (operation) {
            "back" -> performGlobalAction(GLOBAL_ACTION_BACK)
            "home" -> performGlobalAction(GLOBAL_ACTION_HOME)
            "open_app" -> {
                val name = args.getString("packageName")
                require(name.matches(Regex("[A-Za-z0-9_]+(\\.[A-Za-z0-9_]+)+")))
                val intent = packageManager.getLaunchIntentForPackage(name) ?: error("This app cannot be opened.")
                startActivity(intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)); true
            }
            else -> {
                val target = nodes[args.getInteger("nodeId")] ?: error("Select a node from the current observation.")
                require(target.node.refresh() && !target.node.isPassword && target.node.isVisibleToUser && target.node.isEnabled && signature(target.node) == target.signature) { "The target changed; observe again." }
                when(operation) {
                    "click" -> { require(target.node.isClickable); target.node.performAction(AccessibilityNodeInfo.ACTION_CLICK) }
                    "set_text" -> { require(target.node.isEditable); val text = args.getString("text"); require(text.length <= 64000); target.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,text) }) }
                    "scroll_forward" -> target.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)
                    "scroll_backward" -> target.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD)
                    else -> error("Unsupported phone action.")
                }
            }
        }
        invalidate()
        return JSObject().apply { put("actionAccepted",accepted); put("verified",false); put("next","Observe again and verify the outcome.") }
    }
}
