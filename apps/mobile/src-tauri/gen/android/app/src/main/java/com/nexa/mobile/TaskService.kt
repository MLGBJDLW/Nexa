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
        const val NOTIFICATION = 71
        val stopRequested = AtomicBoolean(false)
        private val approvals = mutableMapOf<String, Invoke>()
        private val stopWaiters = mutableListOf<Invoke>()
        @Synchronized fun stopAndWait(activity: Activity, invoke: Invoke) {
            stopWaiters.add(invoke)
            if (!activity.stopService(Intent(activity, TaskService::class.java))) {
                stopWaiters.remove(invoke); invoke.resolve(JSObject())
            }
        }
        @Synchronized fun stopped() { stopWaiters.forEach { it.resolve(JSObject()) }; stopWaiters.clear() }
        @Synchronized fun pending(id: String, invoke: Invoke) { approvals.remove(id)?.resolve(JSObject("{\"allow\":false}")); approvals[id] = invoke }
        @Synchronized fun answer(id: String, allow: Boolean) { approvals.remove(id)?.resolve(JSObject("{\"allow\":$allow}")) }
        @Synchronized fun denyAll(notifications: NotificationManager) { approvals.forEach { (id, invoke) -> notifications.cancel(id.hashCode()); invoke.resolve(JSObject("{\"allow\":false}")) }; approvals.clear() }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.task_channel), NotificationManager.IMPORTANCE_DEFAULT))
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
        val notification = NotificationCompat.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_nexa_notification)
            .setContentTitle(getString(R.string.task_running)).setContentText(getString(R.string.task_progress))
            .setContentIntent(open).setOngoing(true).addAction(0,getString(R.string.task_stop),stop).build()
        startForeground(NOTIFICATION, notification)
        if (wakeLock == null) {
            wakeLock = (getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"Nexa:PhoneTask").apply { acquire() }
        }
        return START_NOT_STICKY
    }
    override fun onDestroy() { PhonePlugin.instance?.taskStopped(); wakeLock?.let { if (it.isHeld) it.release() }; wakeLock = null; denyAll(getSystemService(NotificationManager::class.java)); super.onDestroy(); stopped() }
    override fun onBind(intent: Intent?): IBinder? = null
}
