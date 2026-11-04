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
    override fun onResume(activity: androidx.appcompat.app.AppCompatActivity) { foreground = true }
    override fun onPause(activity: androidx.appcompat.app.AppCompatActivity) { foreground = false; speech.stop() }
    override fun onDestroy(activity: androidx.appcompat.app.AppCompatActivity) { speech.destroy(); if (instance === this) instance = null }
    @Command fun speech(invoke: Invoke) = activity.runOnUiThread {
        when (invoke.getArgs().getString("action")) {
            "listen" -> if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) speech.listen(invoke)
                else requestPermissionForAlias("microphone", invoke, "microphoneGranted")
            "cancel" -> { speech.cancelListening(); invoke.resolve(JSObject()) }
            "speak" -> speech.speak(invoke)
            "stop" -> { speech.stopSpeech(); invoke.resolve(JSObject()) }
            else -> invoke.reject("Unsupported speech operation.")
        }
    }
    @PermissionCallback private fun microphoneGranted(invoke: Invoke) {
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) speech.listen(invoke)
        else invoke.reject("speech_permission")
    }
    private fun guarded(invoke: Invoke, action: () -> JSObject) { try { invoke.resolve(action()) } catch (error: Exception) { invoke.reject(error.message ?: "Native service unavailable.") } }
    @Command fun secureRead(invoke: Invoke) = guarded(invoke) { JSObject().apply { put("value",secrets.read(invoke.getArgs().getString("key"))) } }
    @Command fun scanPairing(invoke: Invoke) {
        try {
            val options = ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE)
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
            check(activity.getSystemService(NotificationManager::class.java).areNotificationsEnabled()) { "Enable task notifications first so Stop remains reachable." }
            TaskService.stopRequested.set(false); ContextCompat.startForegroundService(activity,intent)
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
                TaskService.ensureChannels(activity)
                val open = PendingIntent.getActivity(activity, 2, Intent(activity, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
                activity.getSystemService(NotificationManager::class.java).notify(TaskService.REPLY_NOTIFICATION,
                    NotificationCompat.Builder(activity, TaskService.CHANNEL).setSmallIcon(R.drawable.ic_nexa_notification)
                        .setContentTitle(activity.getString(R.string.task_reply_ready)).setContentText(activity.getString(R.string.task_reply_open))
                        .setContentIntent(open).setAutoCancel(true).setCategory(NotificationCompat.CATEGORY_MESSAGE).build())
            }
            JSObject()
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
