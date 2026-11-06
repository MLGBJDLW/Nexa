package com.nexa.mobile

import android.Manifest
import android.app.*
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import androidx.activity.result.ActivityResult
import com.journeyapps.barcodescanner.ScanOptions
import com.journeyapps.barcodescanner.ScanIntentResult
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")])
class PhonePlugin(private val activity: Activity): Plugin(activity) {
    companion object { var instance: PhonePlugin? = null }
    override fun load(webView: android.webkit.WebView) { instance = this }
    fun taskStopped() { trigger("taskStop",JSObject()) }
    private val secrets = SecretStore(activity)
    private val speech = PhoneSpeech(activity)
    private var foreground = true
    private var pendingListen: Invoke? = null
    private var preparedListenId: String? = null
    private var captureOwnerId: String? = null
    private var permissionListen: Invoke? = null
    override fun onResume(activity: androidx.appcompat.app.AppCompatActivity) {
        foreground = true
        // Discard the single, unscoped reply posted by older app versions.
        activity.getSystemService(NotificationManager::class.java).cancel(TaskService.REPLY_NOTIFICATION)
        pendingListen?.let { pendingListen = null; speech.listen(it) }
    }
    override fun onPause(activity: androidx.appcompat.app.AppCompatActivity) { foreground = false; speech.stop() }
    override fun onDestroy(activity: androidx.appcompat.app.AppCompatActivity) { cancelPendingSpeech("speech_interrupted"); speech.destroy(); if (instance === this) instance = null }
    private fun cancelPendingSpeech(reason: String) {
        pendingListen?.reject(reason); pendingListen = null
        permissionListen?.reject(reason); permissionListen = null
    }
    @Command fun speech(invoke: Invoke) = activity.runOnUiThread {
        val requestId = invoke.getArgs().optString("requestId").takeUnless { it.isEmpty() || it == "null" }
        when (invoke.getArgs().getString("action")) {
            "prepare" -> when {
                requestId == null -> invoke.reject("speech_failed")
                speech.inputBusy || pendingListen != null || permissionListen != null -> invoke.reject("speech_busy")
                else -> { preparedListenId = requestId; invoke.resolve(JSObject()) }
            }
            // A release/cancel can arrive while Rust is resolving a cloud key.
            // Revoking the prepared ID prevents that delayed listen from opening the mic.
            "listen" -> when {
                requestId != null && preparedListenId != requestId -> invoke.reject("speech_cancelled")
                speech.inputBusy || pendingListen != null || permissionListen != null -> invoke.reject("speech_busy")
                else -> {
                    preparedListenId = null; captureOwnerId = requestId
                    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                        if (foreground) speech.listen(invoke) else invoke.reject("speech_interrupted")
                    } else { permissionListen = invoke; requestPermissionForAlias("microphone", invoke, "microphoneGranted") }
                }
            }
            "cancel", "finish" -> {
                if (requestId == null || preparedListenId == requestId) preparedListenId = null
                if (requestId == null || captureOwnerId == requestId) {
                    if (invoke.getArgs().getString("action") == "cancel") {
                        captureOwnerId = null; cancelPendingSpeech("speech_cancelled"); speech.cancelListening()
                    } else if (pendingListen != null || permissionListen != null) cancelPendingSpeech("speech_cancelled") else speech.finishListening()
                }
                invoke.resolve(JSObject())
            }
            "speak" -> speech.speak(invoke)
            "play" -> if (foreground) speech.play(invoke) else invoke.reject("speech_interrupted")
            "stop" -> { speech.stopSpeech(); invoke.resolve(JSObject()) }
            "status" -> guarded(invoke) { speech.status() }
            "configure" -> guarded(invoke) { speech.configure(invoke.getArgs()) }
            "settings" -> guarded(invoke) { PhoneAccess.open(activity, if (invoke.getArgs().getString("mode", "system") == "tts") "tts" else "speech"); JSObject() }
            else -> invoke.reject("Unsupported speech operation.")
        }
    }
    @PermissionCallback private fun microphoneGranted(invoke: Invoke) {
        if (permissionListen !== invoke) return
        permissionListen = null
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            // OEM permission activities can deliver the result before onResume.
            // Starting then immediately cancels recognition in onPause.
            if (foreground) speech.listen(invoke)
            else { pendingListen?.reject("speech_cancelled"); pendingListen = invoke }
        }
        else invoke.reject("speech_permission")
    }
    private fun guarded(invoke: Invoke, action: () -> JSObject) { try { invoke.resolve(action()) } catch (error: Exception) { invoke.reject(error.message ?: "Native service unavailable.") } }
    @Command fun secureRead(invoke: Invoke) = guarded(invoke) { JSObject().apply { put("value",secrets.read(invoke.getArgs().getString("key"))) } }
    @Command fun scanPairing(invoke: Invoke) {
        try {
            val options = ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setCaptureActivity(PairingScannerActivity::class.java)
                .setBeepEnabled(false).setOrientationLocked(false)
            startActivityForResult(invoke, options.createScanIntent(activity), "pairingScanned")
        } catch (error: Exception) { invoke.reject(error.message ?: "The camera could not be opened. Paste the pairing link instead.") }
    }
    @ActivityCallback private fun pairingScanned(invoke: Invoke, result: ActivityResult) = guarded(invoke) {
        val scanned = ScanIntentResult.parseActivityResult(result.resultCode, result.data)
        JSObject().apply { put("url", scanned.contents) }
    }
    @Command fun secureWrite(invoke: Invoke) = guarded(invoke) {
        val args = invoke.getArgs(); secrets.write(args.getString("key"),args.getString("value",null)); JSObject()
    }
    @Command fun capabilities(invoke: Invoke) = guarded(invoke) { PhoneAccess.status(activity) }
    @Command fun requestPermission(invoke: Invoke) = guarded(invoke) {
        when(invoke.getArgs().getString("permission")) {
            "accessibility", "app_info", "battery" -> PhoneAccess.open(activity, invoke.getArgs().getString("permission"))
            "notifications" -> {
                val preferences = activity.getPreferences(Activity.MODE_PRIVATE)
                if (Build.VERSION.SDK_INT >= 33 && !preferences.getBoolean("notificationsRequested", false) &&
                    ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    preferences.edit().putBoolean("notificationsRequested", true).apply()
                    activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS),702)
                } else PhoneAccess.open(activity, "notifications")
            }
            else -> error("Unsupported permission.")
        }; JSObject()
    }
    @Command fun taskService(invoke: Invoke) {
        if (!invoke.getArgs().getBoolean("active")) { TaskService.stopAndWait(activity, invoke); return }
        guarded(invoke) {
        val intent = Intent(activity,TaskService::class.java)
        if(invoke.getArgs().getBoolean("active")) {
            check(TaskService.remoteHost == null) { "Turn off computer phone control before starting a local phone task." }
            check(activity.getSystemService(NotificationManager::class.java).areNotificationsEnabled()) { "Enable task notifications first so Stop remains reachable." }
            TaskService.stopRequested.set(false); TaskService.localTaskActive = true
            try { ContextCompat.startForegroundService(activity,intent) } catch (error: Exception) { TaskService.localTaskActive = false; throw error }
        }
        JSObject()
        }
    }
    @Command fun phoneControl(invoke: Invoke) {
        try {
            if (invoke.getArgs().getString("action") == "status") invoke.resolve(PhoneAccess.status(activity))
            else {
                val service = PhoneAccessibilityService.instance
                if (service == null) invoke.resolve(PhoneAccess.recovery(activity)) else service.execute(invoke)
            }
        } catch (error: Exception) { invoke.reject(error.message ?: "Phone control is unavailable.") }
    }
    @Command fun cancelPhoneControl(invoke: Invoke) {
        PhoneAccessibilityService.instance?.cancelRequest(invoke.getArgs().getString("requestId")); invoke.resolve(JSObject())
    }
    @Command fun remoteControl(invoke: Invoke) = activity.runOnUiThread {
        try {
            val args = invoke.getArgs(); val host = args.getString("hostId", "").orEmpty()
            when (args.getString("action")) {
                "status" -> invoke.resolve(JSObject().apply { put("enabled", TaskService.remoteHost == host && !TaskService.stopRequested.get()) })
                "enable" -> {
                    require(host.isNotBlank() && host.length <= 128) { "Choose a paired computer." }
                    check(!TaskService.localTaskActive) { "Stop the local phone task before enabling computer control." }
                      check(TaskService.remoteHost == null || TaskService.remoteHost == host) { "Turn off the current computer's phone control before enabling another computer." }
                    check(PhoneAccessibilityService.instance != null) { "Enable phone control access first." }
                    check(activity.getSystemService(NotificationManager::class.java).areNotificationsEnabled()) { "Enable task notifications so Stop remains reachable." }
                    TaskService.remoteHost = host; TaskService.stopRequested.set(false)
                    try { ContextCompat.startForegroundService(activity, Intent(activity, TaskService::class.java)) }
                    catch (error: Exception) { TaskService.remoteHost = null; throw error }
                    invoke.resolve(JSObject())
                }
                "disable" -> {
                    if (TaskService.remoteHost == host) { TaskService.remoteHost = null; TaskService.stopRequested.set(true); TaskService.stopAndWait(activity, invoke) }
                    else invoke.resolve(JSObject())
                }
                "cancel" -> { PhoneAccessibilityService.instance?.cancelRequest(args.getString("requestId")); invoke.resolve(JSObject()) }
                "execute" -> {
                    check(TaskService.remoteHost == host && !TaskService.stopRequested.get()) { "Phone control was stopped or is not enabled for this computer." }
                    val service = PhoneAccessibilityService.instance ?: error("Phone control access disconnected.")
                    val operation = JSObject(args.getJSONObject("arguments").toString())
                    if (operation.getString("action") == "status") invoke.resolve(PhoneAccess.status(activity))
                    else service.execute(invoke, operation, args.getString("requestId"))
                }
                else -> error("Unsupported remote control operation.")
            }
        } catch (error: Exception) { invoke.reject(error.message ?: "Remote phone control is unavailable.") }
    }
    @Command fun assistantOverlay(invoke: Invoke) = activity.runOnUiThread {
        guarded(invoke) {
            val enabled = invoke.getArgs().getBoolean("enabled")
            val service = PhoneAccessibilityService.instance
            check(!enabled || service != null) { "Enable phone control access before using the floating assistant." }
            service?.setOverlay(enabled)
            activity.getSharedPreferences("phone-assistant", Activity.MODE_PRIVATE).edit().putBoolean("overlay", enabled).apply()
            JSObject()
        }
    }
    @Command fun shareFile(invoke: Invoke) = guarded(invoke) {
        val file = java.io.File(invoke.getArgs().getString("path")).canonicalFile
        require(file.toPath().startsWith(java.io.File(activity.cacheDir,"shared").canonicalFile.toPath()))
        val uri = androidx.core.content.FileProvider.getUriForFile(activity,"${activity.packageName}.fileprovider",file)
        val intent = Intent(Intent.ACTION_SEND).setType("*/*").putExtra(Intent.EXTRA_STREAM,uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        activity.startActivity(Intent.createChooser(intent,activity.getString(R.string.share_file))); JSObject()
    }
    @Command fun taskResult(invoke: Invoke) = activity.runOnUiThread {
        guarded(invoke) {
            if (!foreground && invoke.getArgs().getString("status") == "completed") {
                PhoneReplies.post(activity, invoke.getArgs().getString("conversationId"), invoke.getArgs().getString("runId"))
            }
            JSObject()
        }
    }
    @Command fun replyRead(invoke: Invoke) = activity.runOnUiThread {
        guarded(invoke) {
            if (foreground) PhoneReplies.read(activity, invoke.getArgs().getString("conversationId"), invoke.getArgs().getString("runId"))
            JSObject()
        }
    }
    @Command fun scheduleWake(invoke: Invoke) = activity.runOnUiThread {
        guarded(invoke) {
            val args = invoke.getArgs()
            PhoneScheduleJobs.reconcile(activity, if (args.isNull("nextWakeAt")) null else args.getString("nextWakeAt"))
            JSObject()
        }
    }
    @Command fun scheduleDecisionRead(invoke: Invoke) = guarded(invoke) {
        PhoneReplies.clearApproval(activity, invoke.getArgs().getString("runId"))
        JSObject()
    }
    @Command fun notificationTarget(invoke: Invoke) = activity.runOnUiThread {
        guarded(invoke) {
            val target = PhoneReplies.consume(activity.intent)
            JSObject().apply { if (target != null) { put("conversationId", target.first); put("schedules", target.second) } }
        }
    }
    @Command fun approval(invoke: Invoke) {
        try {
            val args = invoke.getArgs(); val id = args.getString("id")
            val notifications = activity.getSystemService(NotificationManager::class.java)
            if (args.getBoolean("cancel",false)) { TaskService.answer(id,false); notifications.cancel(id.hashCode()); invoke.resolve(JSObject()); return }
            TaskService.pending(id,invoke)
            fun action(allow: Boolean): PendingIntent = PendingIntent.getService(activity,31*id.hashCode()+if(allow)1 else 0,
                Intent(activity,TaskService::class.java).setAction(if(allow) "allow" else "deny").putExtra("approvalId",id),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            TaskService.ensureChannels(activity)
            notifications.notify(id.hashCode(),NotificationCompat.Builder(activity,TaskService.CHANNEL)
                .setSmallIcon(R.drawable.ic_nexa_notification).setContentTitle(activity.getString(R.string.approval_title))
                .setContentText(args.getString("reason")).setStyle(NotificationCompat.BigTextStyle().bigText(args.getString("reason")))
                .setOngoing(true).setOnlyAlertOnce(true).setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
                .addAction(0,activity.getString(R.string.approval_allow),action(true)).addAction(0,activity.getString(R.string.approval_deny),action(false)).build())
        } catch(error: Exception) { invoke.reject(error.message ?: "Approval notification failed.") }
    }
}
