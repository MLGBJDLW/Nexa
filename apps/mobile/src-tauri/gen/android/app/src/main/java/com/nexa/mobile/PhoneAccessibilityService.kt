package com.nexa.mobile

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.app.KeyguardManager
import android.content.Intent
import android.graphics.Path
import android.graphics.Rect
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import app.tauri.plugin.JSObject
import app.tauri.plugin.Invoke
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

private class PhoneUiUnavailable : IllegalStateException("Android does not expose this screen's accessible UI. Wait for a transition to finish, then observe again. System permission and other protected screens may require the user to operate them manually; accessibility does not need to be re-enabled.")

class PhoneAccessibilityService : AccessibilityService() {
    companion object { @Volatile var instance: PhoneAccessibilityService? = null }
    private data class Target(val node: AccessibilityNodeInfo, val signature: String)
    private val nodes = mutableMapOf<Int, Target>()
    private var observation = ""
    private var observedPackage = ""
    private var observedAt = 0L
    private var observedWindow = -1
    private var revision = 0L
    private var screenshotRevision = -1L
    private var screenshotWidth = 0
    private var screenshotHeight = 0
    private var displayWidth = 0
    private var displayHeight = 0
    private var observedRotation = 0
    private var sensitive = false
    private var lastActionAt = 0L
    private var lastUiEventAt = 0L
    private var approvalResolvedAt = 0L
    private data class ApprovalWait(val id: String, val observation: String, val revision: Long, val started: Long, val resolved: Boolean = false)
    private var approvalWait: ApprovalWait? = null
    private val main = Handler(Looper.getMainLooper())
    private val capture by lazy { PhoneScreenCapture(this) }
    private val overlay by lazy { PhoneAssistantOverlay(this) }
    override fun onServiceConnected() { instance = this; if (getSharedPreferences("phone-assistant", MODE_PRIVATE).getBoolean("overlay", false)) overlay.show() }
    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event != null) lastUiEventAt = android.os.SystemClock.elapsedRealtime()
        if (event != null) {
            val windowChanged = event.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED || event.eventType == AccessibilityEvent.TYPE_WINDOWS_CHANGED
            val contentChanged = event.eventType in listOf(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
                AccessibilityEvent.TYPE_VIEW_SCROLLED, AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED,
                AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED, AccessibilityEvent.TYPE_VIEW_SELECTED,
                AccessibilityEvent.TYPE_VIEW_CLICKED, AccessibilityEvent.TYPE_VIEW_LONG_CLICKED)
            // Our overlay and the explicit approval shade are separate from
            // target content. Foreground identity is checked again on return.
            if ((contentChanged && event.packageName?.toString() == observedPackage && event.windowId == observedWindow)
                || (windowChanged && !foregroundMatchesObservation() && !approvalShadeVisible())) revision++
        }
    }
    override fun onInterrupt() { invalidate() }
    override fun onDestroy() { capture.close(); overlay.hide(); invalidate(); instance = null; super.onDestroy() }
    fun setOverlay(enabled: Boolean) { if (enabled) overlay.show() else overlay.hide() }
    @Suppress("DEPRECATION") private fun rotation() = getSystemService(WindowManager::class.java).defaultDisplay.rotation
    @Suppress("DEPRECATION") private fun foregroundMatchesObservation(): Boolean {
        if (PhoneAccess.screenRecovery(this) != null) return false
        val root = rootInActiveWindow ?: return false
        return try { root.packageName?.toString() == observedPackage && root.windowId == observedWindow }
        finally { root.recycle() }
    }
    @Suppress("DEPRECATION") private fun approvalShadeVisible(): Boolean {
        if (approvalWait == null || PhoneAccess.screenRecovery(this) != null) return false
        val root = rootInActiveWindow ?: return false
        return try { root.packageName?.toString() == "com.android.systemui" } finally { root.recycle() }
    }
    fun beginApproval(id: String) { approvalWait = ApprovalWait(id, observation, revision, android.os.SystemClock.elapsedRealtime()) }
    fun finishApproval(id: String) {
        val wait = approvalWait ?: return
        if (wait.id != id || wait.resolved) return
        // Human review time is not model delay. Any underlying content/app
        // change still invalidates the screenshot, including while waiting.
        if (wait.observation == observation && wait.revision == revision) observedAt += android.os.SystemClock.elapsedRealtime() - wait.started
        approvalResolvedAt = android.os.SystemClock.elapsedRealtime()
        approvalWait = wait.copy(resolved = true)
        main.postDelayed({ if (approvalWait?.id == id && approvalWait?.resolved == true) approvalWait = null }, 1500)
    }
    private fun checkUnlocked() { check(!getSystemService(KeyguardManager::class.java).isKeyguardLocked) { "Unlock the phone before using phone control." } }
    // Approval is an explicit user action. Dismiss its shade before the already
    // approved target is checked; never relax the foreground identity check.
    fun dismissApprovalShade() {
        val root = rootInActiveWindow ?: return
        val shade = root.packageName?.toString() == "com.android.systemui"
        @Suppress("DEPRECATION") root.recycle()
        if (shade) performGlobalAction(if (android.os.Build.VERSION.SDK_INT >= 31) GLOBAL_ACTION_DISMISS_NOTIFICATION_SHADE else GLOBAL_ACTION_BACK)
    }
    @Suppress("DEPRECATION") private fun invalidate() { nodes.values.forEach { it.node.recycle() }; nodes.clear(); observation = ""; screenshotRevision = -1 }
    private fun signature(node: AccessibilityNodeInfo): String {
        val bounds = Rect(); node.getBoundsInScreen(bounds)
        return "${node.windowId}|${node.packageName}|${node.viewIdResourceName}|${node.className}|${node.text}|${node.contentDescription}|${bounds.flattenToString()}|${node.isEnabled}|${node.isPassword}"
    }
    @Synchronized fun observe(): JSObject {
        checkUnlocked()
        invalidate()
        val root = rootInActiveWindow ?: throw PhoneUiUnavailable()
        observedPackage = root.packageName?.toString() ?: error("The active app cannot be identified.")
        observedWindow = root.windowId; observedRotation = rotation(); sensitive = false
        observation = UUID.randomUUID().toString(); observedAt = android.os.SystemClock.elapsedRealtime()
        val result = JSONArray(); var visited = 0; var truncated = false
        fun walk(node: AccessibilityNodeInfo, depth: Int) {
            if (visited++ >= 512 || depth > 40) { truncated = true; return }
            if (node.isPassword || (android.os.Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive)) { sensitive = true; return }
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
        return JSObject().apply { put("observationId",observation); put("packageName",observedPackage); put("nodes",result); put("truncated",truncated); put("sensitiveFieldsOmitted",true); put("containsSensitiveFields",sensitive) }
    }
    fun execute(invoke: Invoke) {
        val started = android.os.SystemClock.elapsedRealtime()
        fun ready() {
            val now = android.os.SystemClock.elapsedRealtime()
            val read = invoke.getArgs().getString("action") in listOf("observe", "screenshot")
            // SystemUI can expose the underlying root before its dismissal
            // animation stops intercepting gestures. Settle explicit approvals
            // before dispatching input, without replaying the input itself.
            val approvalSettling = now - approvalResolvedAt < 1000 || (approvalWait?.resolved == true && !foregroundMatchesObservation())
            if (now-started < 2000 && (approvalSettling || (read && now-started < 1200 && (now-lastActionAt < 350 || now-lastUiEventAt < 200)))) {
                main.postDelayed({ ready() }, 60)
            } else executeNow(invoke)
        }
        main.post { ready() }
    }
    private fun executeNow(invoke: Invoke) {
        try {
            check(instance === this) { "The accessibility service disconnected. Check phone control access." }
            val args = invoke.getArgs()
            PhoneAccess.screenRecovery(this)?.let { invoke.resolve(it); return }
            TaskService.keepControlScreenOn()
            when (args.getString("action")) {
                "screenshot" -> {
                    overlay.suspend()
                    // Give WindowManager a frame to remove our own assistant control.
                    main.postDelayed({
                        try {
                            check(!TaskService.stopRequested.get()) { "The task was stopped." }
                            val state = observe()
                            check(!sensitive && !state.getBoolean("truncated")) { "This screen contains sensitive controls or an incomplete UI tree. Use accessible controls; screenshot input is unavailable." }
                            val id = observation; val version = revision; val screenRotation = observedRotation
                            capture.capture(invoke, state,
                                { observation == id && revision == version && rotation() == screenRotation && !TaskService.stopRequested.get() && instance === this && foregroundMatchesObservation() },
                                { overlay.restore() },
                                { width, height, fullWidth, fullHeight -> screenshotRevision = version; screenshotWidth = width; screenshotHeight = height; displayWidth = fullWidth; displayHeight = fullHeight })
                        } catch (error: Exception) { overlay.restore(); finishError(invoke, error) }
                    }, 120)
                }
                "tap" -> tap(invoke, args)
                "list_apps" -> {
                    val query = args.getString("query", "").orEmpty().lowercase()
                    val apps = packageManager.queryIntentActivities(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0)
                        .distinctBy { it.activityInfo.packageName }
                        .map { it.activityInfo.packageName to it.loadLabel(packageManager).toString() }
                        .filter { query.isEmpty() || it.first.lowercase().contains(query) || it.second.lowercase().contains(query) }
                        .sortedBy { it.second }
                    invoke.resolve(JSObject().apply { put("apps", JSONArray(apps.take(80).map { (pkg, name) -> JSONObject().put("packageName", pkg).put("name", name) })); put("hasMore", apps.size > 80) })
                }
                else -> invoke.resolve(action(args))
            }
        } catch (error: Exception) { finishError(invoke, error) }
    }
    private fun finishError(invoke: Invoke, error: Exception) {
        if (error is PhoneUiUnavailable) invoke.resolve(JSObject().apply {
            put("code", "ui_unavailable"); put("retryable", true); put("requiresUserAction", false)
            put("message", error.message); put("recovery", "Observe once after the transition. If still unavailable, ask the user to operate this protected screen; do not repeat a possibly completed action.")
        }) else invoke.reject(error.message ?: "Phone action is unavailable.")
    }
    private fun validateTarget(args: JSObject) {
        checkUnlocked()
        check(!TaskService.stopRequested.get()) { "The task was stopped." }
        require(args.getString("observationId") == observation && observation.isNotEmpty()) { "Observe the screen again before acting." }
        require(android.os.SystemClock.elapsedRealtime() - observedAt < 60_000) { "The observation expired; observe again." }
        val currentRoot = rootInActiveWindow ?: throw PhoneUiUnavailable()
        val currentPackage = currentRoot.packageName?.toString(); val currentWindow = currentRoot.windowId
        @Suppress("DEPRECATION") currentRoot.recycle()
        require(currentPackage == observedPackage && currentWindow == observedWindow) { "The foreground app changed; observe again." }
        if (approvalWait?.resolved == true) approvalWait = null
        if (args.getString("action") != "open_app") require(args.getString("packageName") == observedPackage) { "The target package must match the observation." }
    }
    private fun tap(invoke: Invoke, args: JSObject) {
        validateTarget(args)
        require(screenshotRevision >= 0 && screenshotRevision == revision && rotation() == observedRotation) { "Take a fresh screenshot before tapping coordinates; the screen changed." }
        require(nodes.values.all { it.node.refresh() && signature(it.node) == it.signature }) { "The observed controls changed during approval; capture again." }
        require(android.os.SystemClock.elapsedRealtime() - observedAt < 10_000) { "The screenshot expired; capture again before tapping." }
        val x = args.getDouble("x"); val y = args.getDouble("y")
        require(x.isFinite() && y.isFinite() && x >= 0 && y >= 0 && x < screenshotWidth && y < screenshotHeight) { "Tap coordinates must be inside the returned screenshot." }
        val pointX = (x * displayWidth / screenshotWidth).toFloat(); val pointY = (y * displayHeight / screenshotHeight).toFloat()
        val path = Path().apply { moveTo(pointX, pointY) }
        val gesture = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path, 0, 80)).build()
        invalidate()
        var answered = false
        fun done(accepted: Boolean) { if (!answered) { answered = true; lastActionAt = android.os.SystemClock.elapsedRealtime(); invoke.resolve(JSObject().apply { put("actionAccepted", accepted); put("verified", false); put("next", "Observe again and verify the outcome. Never repeat an uncertain send action.") }) } }
        val dispatched = dispatchGesture(gesture, object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) { done(true) }
            override fun onCancelled(gestureDescription: GestureDescription?) { done(false) }
        }, main)
        if (!dispatched) done(false)
        main.postDelayed({ if (!answered) { answered = true; invoke.reject("The gesture outcome is unknown. Observe before deciding what to do; do not repeat a possible send.") } }, 5000)
    }
    @Synchronized fun action(args: JSObject): JSObject {
        check(!TaskService.stopRequested.get()) { "The task was stopped." }
        val operation = args.getString("action")
        if (operation == "observe") return observe()
        validateTarget(args)
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
        lastActionAt = android.os.SystemClock.elapsedRealtime()
        invalidate()
        return JSObject().apply { put("actionAccepted",accepted); put("verified",false); put("next","Observe again and verify the outcome.") }
    }
}
