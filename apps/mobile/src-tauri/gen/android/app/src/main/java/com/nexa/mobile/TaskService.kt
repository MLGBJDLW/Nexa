package com.nexa.mobile

import android.app.*
import android.content.Intent
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import java.util.concurrent.atomic.AtomicBoolean
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

class TaskService : Service() {
    companion object {
        const val CHANNEL = "nexa_phone_tasks"
        const val STATUS_CHANNEL = "nexa_phone_task_status_v2"
        const val NOTIFICATION = 71
        const val REPLY_NOTIFICATION = 72
        fun ensureChannels(context: android.content.Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            // Keep the existing attention channel's user settings. A new low
            // importance channel is needed because Android cannot downgrade an
            // already-created channel's importance programmatically.
            manager.createNotificationChannel(NotificationChannel(CHANNEL, context.getString(R.string.task_attention_channel), NotificationManager.IMPORTANCE_DEFAULT))
            manager.createNotificationChannel(NotificationChannel(STATUS_CHANNEL, context.getString(R.string.task_channel), NotificationManager.IMPORTANCE_LOW).apply { setSound(null, null); enableVibration(false); setShowBadge(false) })
        }
        val stopRequested = AtomicBoolean(false)
        @Volatile private var activeService: TaskService? = null
        fun keepControlScreenOn() { activeService?.holdControlScreen() }
        private val approvals = mutableMapOf<String, Invoke>()
        private val stopWaiters = mutableListOf<Invoke>()
        @Synchronized fun stopAndWait(activity: Activity, invoke: Invoke) {
            stopWaiters.add(invoke)
            if (!activity.stopService(Intent(activity, TaskService::class.java))) {
                stopWaiters.remove(invoke); invoke.resolve(JSObject())
            }
        }
        @Synchronized fun stopped() { stopWaiters.forEach { it.resolve(JSObject()) }; stopWaiters.clear() }
        @Synchronized fun pending(id: String, invoke: Invoke) { approvals.remove(id)?.resolve(JSObject("{\"allow\":false}")); approvals[id] = invoke; PhoneAccessibilityService.instance?.beginApproval(id) }
        @Synchronized fun answer(id: String, allow: Boolean) {
            val pending = approvals.remove(id) ?: return
            PhoneAccessibilityService.instance?.finishApproval(id)
            if (allow) PhoneAccessibilityService.instance?.dismissApprovalShade()
            pending.resolve(JSObject("{\"allow\":$allow}"))
        }
        @Synchronized fun denyAll(notifications: NotificationManager) { approvals.forEach { (id, invoke) -> PhoneAccessibilityService.instance?.finishApproval(id); notifications.cancel(id.hashCode()); invoke.resolve(JSObject("{\"allow\":false}")) }; approvals.clear() }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    private var controlWakeLock: PowerManager.WakeLock? = null
    @Suppress("DEPRECATION") private fun holdControlScreen() {
        val power = getSystemService(PowerManager::class.java)
        if (controlWakeLock == null && power.isInteractive && !stopRequested.get()) {
            // Keep an already-unlocked display visible during the user's task.
            // No ACQUIRE_CAUSES_WAKEUP: a deliberate lock still stops input.
            controlWakeLock = power.newWakeLock(PowerManager.SCREEN_DIM_WAKE_LOCK, "Nexa:PhoneControl").apply { acquire() }
        }
    }
    override fun onCreate() {
        super.onCreate()
        activeService = this
        ensureChannels(this)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            "stop" -> { stopRequested.set(true); PhonePlugin.instance?.taskStopped(); denyAll(getSystemService(NotificationManager::class.java)); stopSelf(); return START_NOT_STICKY }
            "allow", "deny" -> {
                val id = intent.getStringExtra("approvalId") ?: return START_NOT_STICKY
                answer(id, intent.action == "allow")
                getSystemService(NotificationManager::class.java).cancel(id.hashCode())
                return START_NOT_STICKY
            }
        }
        val stop = PendingIntent.getService(this, 1, Intent(this, TaskService::class.java).setAction("stop"), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val open = PendingIntent.getActivity(this, 2, Intent(this, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        getSystemService(NotificationManager::class.java).cancel(REPLY_NOTIFICATION)
        val notification = NotificationCompat.Builder(this, STATUS_CHANNEL).setSmallIcon(R.drawable.ic_nexa_notification)
            .setContentTitle(getString(R.string.task_running)).setContentText(getString(R.string.task_progress))
            .setSilent(true).setOnlyAlertOnce(true).setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(open).setOngoing(true).addAction(0,getString(R.string.task_stop),stop).build()
        startForeground(NOTIFICATION, notification)
        if (wakeLock == null) {
            wakeLock = (getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"Nexa:PhoneTask").apply { acquire() }
        }
        return START_NOT_STICKY
    }
    override fun onDestroy() { activeService = null; stopRequested.set(true); PhonePlugin.instance?.taskStopped(); controlWakeLock?.let { if (it.isHeld) it.release() }; controlWakeLock = null; wakeLock?.let { if (it.isHeld) it.release() }; wakeLock = null; denyAll(getSystemService(NotificationManager::class.java)); super.onDestroy(); stopped() }
    override fun onBind(intent: Intent?): IBinder? = null
}
