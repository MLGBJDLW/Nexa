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
import java.util.concurrent.Executors

private class PhoneUiUnavailable : IllegalStateException("Android does not expose this screen's accessible UI. Wait for a transition to finish, then observe again. System permission and other protected screens may require the user to operate them manually; accessibility does not need to be re-enabled.")
private class PhoneStale(message: String) : IllegalStateException(message)

class PhoneAccessibilityService : AccessibilityService() {
    companion object { @Volatile var instance: PhoneAccessibilityService? = null }
    private data class Target(val node: AccessibilityNodeInfo, val signature: String, val focused: Boolean)
    private val nodes = mutableMapOf<Int, Target>()
    private var observation = ""
    private var observedPackage = ""
    private var observedAt = 0L
    private var observedWindow = -1
    private var revision = 0L
    private var screenshotRevision = -1L
    private var screenshotFingerprint: String? = null
    private val observedBounds = Rect()
    private var screenshotWidth = 0
    private var screenshotHeight = 0
    private var displayWidth = 0
    private var displayHeight = 0
    private var observedRotation = 0
    private var sensitive = false
    private var observedEditor: android.view.inputmethod.EditorInfo? = null
    private var observedEditorSignature: String? = null
    private var lastActionAt = 0L
    private var lastUiEventAt = 0L
    private var approvalResolvedAt = 0L
    private data class ApprovalWait(val id: String, val observation: String, val revision: Long, val started: Long, val resolved: Boolean = false)
    private var approvalWait: ApprovalWait? = null
    private val main = Handler(Looper.getMainLooper())
    private val capture by lazy { PhoneScreenCapture(this) }
    private val overlay by lazy { PhoneAssistantOverlay(this) }
    private val nativeInputWorker = Executors.newSingleThreadExecutor()
    private var activeRequest: Invoke? = null
    private var activeRequestId: String? = null
    override fun onServiceConnected() {
        instance = this
        if (android.os.Build.VERSION.SDK_INT >= 33) serviceInfo = serviceInfo.apply { flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_INPUT_METHOD_EDITOR }
        if (getSharedPreferences("phone-assistant", MODE_PRIVATE).getBoolean("overlay", false)) overlay.show()
    }
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
    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) { super.onConfigurationChanged(newConfig); invalidate(); overlay.reposition() }
    override fun onDestroy() {
        activeRequest?.let { reply(it, JSObject().apply { put("code", "control_disconnected"); put("retryable", false); put("message", "Phone control disconnected. Observe after reconnecting; do not repeat an uncertain action.") }) }
        capture.close(); nativeInputWorker.shutdownNow(); main.removeCallbacksAndMessages(null); overlay.hide(); invalidate(); instance = null; super.onDestroy()
    }
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
    @Suppress("DEPRECATION") private fun invalidate() { nodes.values.forEach { it.node.recycle() }; nodes.clear(); observation = ""; screenshotRevision = -1; screenshotFingerprint = null; observedEditor = null; observedEditorSignature = null }
    private fun isProtected(node: AccessibilityNodeInfo) = node.isPassword || (android.os.Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive)
    private fun signature(node: AccessibilityNodeInfo): String {
        val bounds = Rect(); node.getBoundsInScreen(bounds)
        val protected = android.os.Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive
        return "${node.windowId}|${node.packageName}|${node.viewIdResourceName}|${node.className}|${node.text}|${node.contentDescription}|${bounds.flattenToString()}|${node.isEnabled}|${node.isPassword}|$protected"
    }
    @Synchronized fun observe(): JSObject {
        checkUnlocked()
        invalidate()
        val root = rootInActiveWindow ?: throw PhoneUiUnavailable()
        observedPackage = root.packageName?.toString() ?: error("The active app cannot be identified.")
        observedWindow = root.windowId; observedRotation = rotation(); sensitive = false
        root.getBoundsInScreen(observedBounds)
        observation = UUID.randomUUID().toString(); observedAt = android.os.SystemClock.elapsedRealtime()
        val result = JSONArray(); var visited = 0; var truncated = false
        fun walk(node: AccessibilityNodeInfo, depth: Int) {
            if (visited++ >= 512 || depth > 40) { truncated = true; return }
            if (node.isPassword || (android.os.Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive)) { sensitive = true; return }
            if (node.isVisibleToUser && !node.isPassword) {
                val id = nodes.size
                val bounds = Rect(); node.getBoundsInScreen(bounds)
                @Suppress("DEPRECATION") val copy = AccessibilityNodeInfo.obtain(node)
                nodes[id] = Target(copy, signature(node), node.isFocused)
                result.put(JSONObject().put("nodeId",id).put("text",node.text?.toString()?.take(2000) ?: "")
                    .put("description",node.contentDescription?.toString()?.take(1000) ?: "").put("class",node.className)
                    .put("clickable",node.isClickable).put("editable",node.isEditable).put("focused",node.isFocused).put("scrollable",node.isScrollable)
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
        if (android.os.Build.VERSION.SDK_INT >= 33 && safeNativeEditor()) {
            observedEditor = inputMethod?.currentInputEditorInfo
            observedEditorSignature = observedEditor?.let { editorSignature(it) }
        }
        return JSObject().apply { put("observationId",observation); put("packageName",observedPackage); put("nodes",result); put("truncated",truncated); put("sensitiveFieldsOmitted",true); put("containsSensitiveFields",sensitive); put("nativeTextInput", safeNativeEditor()) }
    }
    private fun reply(invoke: Invoke, value: JSObject) {
        if (activeRequest !== invoke) return
        activeRequest = null; activeRequestId = null; overlay.restore(); invoke.resolve(value)
    }
    fun cancelAllRequests() {
        fun cancel() {
            activeRequest?.let { request ->
                reply(request, JSObject().apply { put("code", "phone_control_cancelled"); put("retryable", false); put("actionOutcomeUnknown", true); put("message", "Phone control stopped. Observe before continuing; never repeat an uncertain send.") })
            }
            invalidate(); capture.cancelPending()
        }
        if (Looper.myLooper() == Looper.getMainLooper()) cancel() else main.post { cancel() }
    }
    fun cancelRequest(id: String) = main.post {
        if (activeRequestId == id) activeRequest?.let { request ->
            reply(request, JSObject().apply { put("code", "phone_control_cancelled"); put("retryable", false); put("actionOutcomeUnknown", true); put("message", "Phone operation cancelled. Observe before continuing; never repeat an uncertain send.") })
            invalidate(); capture.cancelPending()
        }
    }
    fun execute(invoke: Invoke, arguments: JSObject = invoke.getArgs(), requestId: String? = arguments.getString("requestId", null)) {
        val started = android.os.SystemClock.elapsedRealtime()
        fun ready() {
            val now = android.os.SystemClock.elapsedRealtime()
            val read = arguments.getString("action") in listOf("observe", "screenshot")
            // SystemUI can expose the underlying root before its dismissal
            // animation stops intercepting gestures. Settle explicit approvals
            // before dispatching input, without replaying the input itself.
            val approvalSettling = now - approvalResolvedAt < 1000 || (approvalWait?.resolved == true && !foregroundMatchesObservation())
            if (now-started < 2000 && (approvalSettling || (read && now-started < 1200 && (now-lastActionAt < 350 || now-lastUiEventAt < 200)))) {
                main.postDelayed({ ready() }, 60)
            } else if (activeRequest === invoke) executeNow(invoke, arguments)
        }
        main.post {
            if (activeRequest != null) invoke.resolve(JSObject().apply { put("code", "control_busy"); put("retryable", true); put("actionPerformed", false); put("message", "Another phone operation is still finishing. Wait for its result before acting.") })
            else { activeRequest = invoke; activeRequestId = requestId; ready() }
        }
    }
    private fun executeNow(invoke: Invoke, args: JSObject) {
        try {
            check(instance === this) { "The accessibility service disconnected. Check phone control access." }
            PhoneAccess.screenRecovery(this)?.let { reply(invoke, it); return }
            TaskService.keepControlScreenOn()
            when (args.getString("action")) {
                "screenshot" -> captureObservation { result -> result.fold({ reply(invoke, it.state) }, { finishError(invoke, it) }) }
                "tap" -> tap(invoke, args)
                "set_text" -> setText(invoke, args)
                "list_apps" -> {
                    val query = args.getString("query", "").orEmpty().lowercase()
                    val apps = packageManager.queryIntentActivities(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0)
                        .distinctBy { it.activityInfo.packageName }
                        .map { it.activityInfo.packageName to it.loadLabel(packageManager).toString() }
                        .filter { query.isEmpty() || it.first.lowercase().contains(query) || it.second.lowercase().contains(query) }
                        .sortedBy { it.second }
                    reply(invoke, JSObject().apply { put("apps", JSONArray(apps.take(80).map { (pkg, name) -> JSONObject().put("packageName", pkg).put("name", name) })); put("hasMore", apps.size > 80) })
                }
                "observe" -> reply(invoke, observe())
                else -> completeAction(invoke, action(args))
            }
        } catch (error: Exception) { finishError(invoke, error) }
    }
    private fun finishError(invoke: Invoke, error: Throwable) {
        if (activeRequest !== invoke) return
        val stale = error is PhoneStale || error is PhoneCaptureChanged
        val state = if (stale) try { observe() } catch (_: Exception) { JSObject() } else JSObject()
        state.put("code", if (stale) "stale_observation" else if (error is PhoneUiUnavailable) "ui_unavailable" else "phone_action_unavailable")
        state.put("retryable", stale || error is PhoneUiUnavailable); state.put("requiresUserAction", false); state.put("actionPerformed", false)
        state.put("message", error.message ?: "Phone action is unavailable.")
        state.put("recovery", if (stale) "Inspect this new observation and use its observationId and nodes. For coordinates, capture a new screenshot. No input was sent." else "Inspect phone status and the current screen before deciding what to do; never repeat an uncertain send.")
        reply(invoke, state)
    }
    private fun validateTarget(args: JSObject) {
        checkUnlocked()
        check(!TaskService.stopRequested.get()) { "The task was stopped." }
        if (args.getString("observationId") != observation || observation.isEmpty()) throw PhoneStale("Inspect the current observation before acting.")
        val currentRoot = rootInActiveWindow ?: throw PhoneUiUnavailable()
        val currentPackage = currentRoot.packageName?.toString(); val currentWindow = currentRoot.windowId
        @Suppress("DEPRECATION") currentRoot.recycle()
        if (currentPackage != observedPackage || currentWindow != observedWindow || rotation() != observedRotation) throw PhoneStale("The foreground app or display changed.")
        if (approvalWait?.resolved == true) approvalWait = null
        if (args.getString("action") != "open_app") require(args.getString("packageName") == observedPackage) { "The target package must match the observation." }
    }
    private fun captureObservation(complete: (Result<PhoneCapture>) -> Unit) {
        val request = activeRequest
        overlay.suspend()
        main.postDelayed({
            if (activeRequest !== request) return@postDelayed
            try {
                check(instance === this && !TaskService.stopRequested.get()) { "Phone control stopped." }
                val state = observe()
                check(!sensitive && !state.getBoolean("truncated")) { "This screen contains sensitive controls or an incomplete UI tree. Use accessible controls." }
                val id = observation; val version = revision; val screenRotation = observedRotation
                capture.capture(state, Rect(observedBounds),
                    { activeRequest === request && observation == id && revision == version && rotation() == screenRotation && !TaskService.stopRequested.get() && instance === this && foregroundMatchesObservation() }) { result ->
                    if (activeRequest !== request) return@capture
                    result.onSuccess { frame ->
                        screenshotRevision = version; screenshotFingerprint = frame.fingerprint
                        screenshotWidth = frame.width; screenshotHeight = frame.height; displayWidth = frame.displayWidth; displayHeight = frame.displayHeight
                    }
                    complete(result)
                }
            } catch (error: Exception) { complete(Result.failure(error)) }
        }, 120)
    }

    private fun completeAction(invoke: Invoke, accepted: Boolean, pixels: Boolean = false, extra: JSObject = JSObject()) {
        if (activeRequest !== invoke) return
        lastActionAt = android.os.SystemClock.elapsedRealtime()
        invalidate()
        val started = lastActionAt
        fun finish(state: JSObject) {
            state.put("actionAccepted", accepted); state.put("verified", extra.optBoolean("textMatched", false))
            state.put("next", "Inspect this post-action observation and use its new observationId for the next operation. Never repeat an uncertain send.")
            extra.keys().forEach { key -> state.put(key, extra.get(key)) }
            reply(invoke, state)
        }
        fun settled() {
            if (activeRequest !== invoke) return
            val now = android.os.SystemClock.elapsedRealtime()
            if (now - started < 1500 && (now - started < 300 || now - lastUiEventAt < 180)) { main.postDelayed({ settled() }, 60); return }
            if (pixels && android.os.Build.VERSION.SDK_INT >= 30) captureObservation { result ->
                result.fold({ finish(it.state) }, { finish(try { observe() } catch (_: Exception) { JSObject().apply { put("observationPending", true) } }) })
            } else finish(try { observe() } catch (_: Exception) { JSObject().apply { put("observationPending", true) } })
        }
        settled()
    }

    private fun tap(invoke: Invoke, args: JSObject) {
        validateTarget(args)
        val fingerprint = screenshotFingerprint ?: throw PhoneStale("Capture a screenshot before choosing coordinates.")
        val expectedPackage = observedPackage; val expectedWindow = observedWindow; val expectedRotation = observedRotation
        val expectedControls = nodes.values.map { it.signature }
        val x = args.getDouble("x"); val y = args.getDouble("y")
        require(x.isFinite() && y.isFinite() && x >= 0 && y >= 0 && x < screenshotWidth && y < screenshotHeight) { "Tap coordinates must be inside the returned screenshot." }
        val pointX = (x * displayWidth / screenshotWidth).toFloat(); val pointY = (y * displayHeight / screenshotHeight).toFloat()
        // Revalidate pixels on every coordinate action. There is no model-time
        // deadline, but changed content never inherits an old tap authorization.
        captureObservation { result -> result.fold({ frame ->
            if (activeRequest !== invoke) return@fold
            if (fingerprint != frame.fingerprint || observedPackage != expectedPackage || observedWindow != expectedWindow || observedRotation != expectedRotation
                || expectedControls != nodes.values.map { it.signature }) {
                frame.state.put("code", "stale_observation"); frame.state.put("retryable", true); frame.state.put("actionPerformed", false)
                frame.state.put("change", if (observedPackage != expectedPackage || observedWindow != expectedWindow || observedRotation != expectedRotation) "foreground" else if (expectedControls != nodes.values.map { it.signature }) "controls" else "pixels")
                frame.state.put("message", "The screen changed. Inspect this new screenshot and choose the target again; no input was sent.")
                reply(invoke, frame.state); return@fold
            }
            try {
                checkUnlocked(); check(!TaskService.stopRequested.get() && foregroundMatchesObservation()) { "Phone control stopped or changed target." }
                val path = Path().apply { moveTo(pointX, pointY) }
                val gesture = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path, 0, 80)).build()
                var answered = false
                val timeout = Runnable {
                    if (!answered) { answered = true; completeAction(invoke, false, extra = JSObject().apply { put("code", "action_outcome_unknown"); put("retryable", false); put("actionPerformed", true) }) }
                }
                fun done(accepted: Boolean, uncertain: Boolean) {
                    if (answered) return
                    answered = true; main.removeCallbacks(timeout)
                    completeAction(invoke, accepted, pixels = true, extra = JSObject().apply {
                        if (uncertain) { put("code", "action_outcome_unknown"); put("retryable", false); put("actionPerformed", true) }
                    })
                }
                invalidate(); main.postDelayed(timeout, 5000)
                if (!dispatchGesture(gesture, object : GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription?) { done(true, false) }
                    override fun onCancelled(gestureDescription: GestureDescription?) { done(false, true) }
                }, main)) done(false, false)
            } catch (error: Exception) { finishError(invoke, error) }
        }, { finishError(invoke, it) }) }
    }

    private fun target(args: JSObject): Target {
        val target = nodes[args.getInteger("nodeId")] ?: throw PhoneStale("Select a node from the current observation.")
        validateNode(target)
        return target
    }
    private fun validateNode(target: Target) {
        if (!target.node.refresh() || isProtected(target.node) || !target.node.isVisibleToUser || !target.node.isEnabled || signature(target.node) != target.signature) throw PhoneStale("The target control changed.")
    }
    private fun editorSignature(editor: android.view.inputmethod.EditorInfo) = "${editor.packageName}|${editor.fieldId}|${editor.fieldName}|${editor.inputType}|${editor.imeOptions}"
    private fun observedEditorMatches(): Boolean {
        if (android.os.Build.VERSION.SDK_INT < 33 || !safeNativeEditor()) return false
        val current = inputMethod?.currentInputEditorInfo ?: return false
        // Android returns a new InputConnection wrapper on every getter call,
        // but retains this EditorInfo object until the editor starts/restarts.
        return current === observedEditor && editorSignature(current) == observedEditorSignature
    }
    private fun safeNativeEditor(): Boolean {
        if (android.os.Build.VERSION.SDK_INT < 33 || sensitive) return false
        val editor = inputMethod?.currentInputEditorInfo ?: return false
        if (editor.packageName != observedPackage || inputMethod?.currentInputConnection == null) return false
        val type = editor.inputType
        val variation = type and android.text.InputType.TYPE_MASK_VARIATION
        return when (type and android.text.InputType.TYPE_MASK_CLASS) {
            android.text.InputType.TYPE_CLASS_TEXT -> variation !in listOf(android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD, android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD, android.text.InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD)
            android.text.InputType.TYPE_CLASS_NUMBER -> variation != android.text.InputType.TYPE_NUMBER_VARIATION_PASSWORD
            else -> true
        }
    }
    private fun setText(invoke: Invoke, args: JSObject) {
        validateTarget(args)
        require(args.has("text") && !args.isNull("text")) { "Provide the text to enter." }
        val text = args.getString("text"); require(text.length <= 64_000) { "Enter at most 64000 characters at a time." }
        val target = if (args.has("nodeId") && !args.isNull("nodeId")) target(args)
            else nodes.values.firstOrNull { it.focused && it.node.isEditable }?.also {
                validateNode(it)
                if (!it.node.isFocused) throw PhoneStale("The focused editor changed. Observe again before entering text.")
            }
        if (target != null) {
            require(target.node.isEditable && !isProtected(target.node)) { "Choose a non-sensitive editable control." }
            if (target.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text) })) {
                verifyText(invoke, target, text, false); return
            }
            if (!target.focused || !target.node.isFocused) throw PhoneStale("Click the editor and inspect the new observation before using native text input.")
        }
        if (android.os.Build.VERSION.SDK_INT < 33) throw IllegalStateException("The app rejected accessibility text input. Native editor fallback requires Android 13+.")
        if (!foregroundMatchesObservation() || TaskService.stopRequested.get()) throw PhoneStale("The input target changed.")
        if (!observedEditorMatches()) throw PhoneStale("The native editor changed. Observe again before entering text.")
        if (target != null) {
            validateNode(target)
            if (!target.node.isFocused) throw PhoneStale("The focused editor changed.")
        }
        val connection = inputMethod?.currentInputConnection ?: throw PhoneStale("The native editor disconnected.")
        connection.performContextMenuAction(android.R.id.selectAll)
        connection.commitText(text, 1, null)
        verifyText(invoke, target, text, true)
    }
    private fun verifyText(invoke: Invoke, target: Target?, expected: String, native: Boolean) {
        val started = android.os.SystemClock.elapsedRealtime()
        fun finish(matched: Boolean) = completeAction(invoke, true, extra = JSObject().apply {
            put("textMatched", matched); put("inputMethod", if (native) "native_input_connection" else "accessibility_set_text")
            if (!matched) { put("code", "text_not_verified"); put("retryable", false); put("actionPerformed", true); put("message", "Text input was attempted but readback did not match. Inspect the returned screen before proceeding.") }
        })
        fun check() {
            if (activeRequest !== invoke) return
            if (!foregroundMatchesObservation() || TaskService.stopRequested.get()) { finish(false); return }
            if (target != null && target.node.refresh() && !isProtected(target.node) && target.node.text?.toString() == expected) { finish(true); return }
            if (target != null && android.os.SystemClock.elapsedRealtime() - started < 1500) { main.postDelayed({ check() }, 80); return }
            if (native && android.os.Build.VERSION.SDK_INT >= 33 && safeNativeEditor()) {
                val connection = inputMethod?.currentInputConnection
                // Reading an editor is an IPC round trip. Keep it off the UI thread.
                nativeInputWorker.execute {
                    val readback = try { connection?.getSurroundingText(64_000, 64_000, 0) } catch (_: Exception) { null }
                    main.post { if (activeRequest === invoke) finish(foregroundMatchesObservation() && safeNativeEditor() && readback?.offset == 0 && readback.text.toString() == expected) }
                }
            } else finish(false)
        }
        main.postDelayed({ check() }, 180)
    }

    private fun action(args: JSObject): Boolean {
        check(!TaskService.stopRequested.get()) { "The task was stopped." }
        val operation = args.getString("action")
        validateTarget(args)
        return when (operation) {
            "back" -> performGlobalAction(GLOBAL_ACTION_BACK)
            "home" -> performGlobalAction(GLOBAL_ACTION_HOME)
            "open_app" -> {
                val name = args.getString("packageName")
                require(name.matches(Regex("[A-Za-z0-9_]+(\\.[A-Za-z0-9_]+)+")))
                val intent = packageManager.getLaunchIntentForPackage(name) ?: error("This app cannot be opened.")
                startActivity(intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)); true
            }
            else -> {
                val target = target(args)
                when(operation) {
                    "click" -> { require(target.node.isClickable); target.node.performAction(AccessibilityNodeInfo.ACTION_CLICK) }
                    "scroll_forward" -> target.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)
                    "scroll_backward" -> target.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD)
                    else -> error("Unsupported phone action.")
                }
            }
        }
    }
}
