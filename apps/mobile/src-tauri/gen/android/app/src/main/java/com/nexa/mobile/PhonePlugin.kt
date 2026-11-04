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
import androidx.activity.result.ActivityResult
import com.journeyapps.barcodescanner.ScanOptions
import com.journeyapps.barcodescanner.ScanIntentResult
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

@TauriPlugin
class PhonePlugin(private val activity: Activity): Plugin(activity) {
    companion object { var instance: PhonePlugin? = null }
    override fun load(webView: android.webkit.WebView) { instance = this }
    fun taskStopped() { trigger("taskStop",JSObject()) }
    private val secrets = SecretStore(activity)
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
    @Command fun phoneControl(invoke: Invoke) = guarded(invoke) {
        if (invoke.getArgs().getString("action") == "status") PhoneAccess.status(activity)
        else PhoneAccessibilityService.instance?.action(invoke.getArgs()) ?: PhoneAccess.recovery(activity)
    }
    @Command fun shareFile(invoke: Invoke) = guarded(invoke) {
        val file = java.io.File(invoke.getArgs().getString("path")).canonicalFile
        require(file.toPath().startsWith(java.io.File(activity.cacheDir,"shared").canonicalFile.toPath()))
        val uri = androidx.core.content.FileProvider.getUriForFile(activity,"${activity.packageName}.fileprovider",file)
        val intent = Intent(Intent.ACTION_SEND).setType("*/*").putExtra(Intent.EXTRA_STREAM,uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        activity.startActivity(Intent.createChooser(intent,activity.getString(R.string.share_file))); JSObject()
    }
    @Command fun approval(invoke: Invoke) {
        try {
            val args = invoke.getArgs(); val id = args.getString("id")
            val notifications = activity.getSystemService(NotificationManager::class.java)
            if (args.getBoolean("cancel",false)) { TaskService.answer(id,false); notifications.cancel(id.hashCode()); invoke.resolve(JSObject()); return }
            TaskService.pending(id,invoke)
            fun action(allow: Boolean): PendingIntent = PendingIntent.getService(activity,31*id.hashCode()+if(allow)1 else 0,
                Intent(activity,TaskService::class.java).setAction(if(allow) "allow" else "deny").putExtra("approvalId",id),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            notifications.notify(id.hashCode(),NotificationCompat.Builder(activity,TaskService.CHANNEL)
                .setSmallIcon(R.drawable.ic_nexa_notification).setContentTitle(activity.getString(R.string.approval_title))
                .setContentText(args.getString("reason")).setStyle(NotificationCompat.BigTextStyle().bigText(args.getString("reason")))
                .setOngoing(true).addAction(0,activity.getString(R.string.approval_allow),action(true)).addAction(0,activity.getString(R.string.approval_deny),action(false)).build())
        } catch(error: Exception) { invoke.reject(error.message ?: "Approval notification failed.") }
    }
}
