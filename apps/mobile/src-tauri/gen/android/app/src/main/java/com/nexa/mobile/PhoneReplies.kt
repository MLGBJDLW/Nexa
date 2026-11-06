package com.nexa.mobile

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.core.app.NotificationCompat
import org.json.JSONObject

/** Chat-scoped delivery from either the visible executor or an Android job. */
internal object PhoneReplies {
    private const val APPROVAL = 73
    private const val OPEN = "com.nexa.mobile.OPEN_REPLY"
    private var viewedRun: String? = null
    private var approvalSequence = 0L

    fun post(context: Context, chat: String, run: String, approval: Boolean = false) {
        if (!approval && viewedRun == run) return
        TaskService.ensureChannels(context)
        val intent = Intent(context, MainActivity::class.java).setAction(OPEN)
            .setData(Uri.Builder().scheme("nexa-internal").authority(if (approval) "schedules" else "chat").appendPath(chat).build())
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val open = PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.notify(if (approval) run else chat, if (approval) APPROVAL else TaskService.REPLY_NOTIFICATION,
            NotificationCompat.Builder(context, TaskService.CHANNEL).setSmallIcon(R.drawable.ic_nexa_notification)
                .setOnlyAlertOnce(approval)
                .addExtras(Bundle().apply { putString("nexa.run", run) })
                .setContentTitle(context.getString(if (approval) R.string.approval_title else R.string.task_reply_ready))
                .setContentText(context.getString(if (approval) R.string.task_schedule_open else R.string.task_reply_open))
                .setContentIntent(open).setAutoCancel(true).setCategory(NotificationCompat.CATEGORY_MESSAGE).build())
    }

    fun read(context: Context, chat: String, run: String) {
        viewedRun = run
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.activeNotifications.any { it.id == TaskService.REPLY_NOTIFICATION && it.tag == chat && it.notification.extras.getString("nexa.run") == run }) {
            manager.cancel(chat, TaskService.REPLY_NOTIFICATION)
        }
    }

    // All callers run on the main looper. Keep receipts even after the user
    // swipes an alert away, until SQLite says the occurrence is no longer pending.
    fun reconcileApprovals(context: Context, snapshot: JSONObject) {
        val pending = snapshot.optJSONObject("pendingApprovals") ?: return
        val sequence = snapshot.getLong("approvalSequence")
        if (sequence <= approvalSequence) return
        val initial = approvalSequence == 0L
        val prefs = context.getSharedPreferences("nexa-schedule-notifications", Context.MODE_PRIVATE)
        val delivered = prefs.getStringSet("delivered", emptySet()).orEmpty()
        val runs = pending.keys().asSequence().toSet()
        if (!initial && runs == delivered) {
            approvalSequence = sequence
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        val active = manager.activeNotifications.filter { it.id == APPROVAL }.mapNotNull { it.tag }.toSet()
        // A just-enqueued alert may not have reached activeNotifications yet.
        for (run in (active + delivered) - runs) manager.cancel(run, APPROVAL)
        val receipts = (delivered + active).intersect(runs).toMutableSet()
        if (runs.size > receipts.size && manager.areNotificationsEnabled()) {
            TaskService.ensureChannels(context)
            if (manager.getNotificationChannel(TaskService.CHANNEL).importance != NotificationManager.IMPORTANCE_NONE) {
                for (run in runs - receipts) {
                    post(context, pending.getString(run), run, approval = true)
                    receipts.add(run)
                }
            }
        }
        if (receipts != delivered) prefs.edit().putStringSet("delivered", receipts).apply()
        approvalSequence = sequence
    }

    fun consume(intent: Intent?): Pair<String, Boolean>? {
        if (intent?.action != OPEN) return null
        val uri = intent.data ?: return null
        intent.action = null
        val chat = uri.lastPathSegment ?: return null
        if (uri.scheme != "nexa-internal" || uri.host !in setOf("chat", "schedules") || runCatching { java.util.UUID.fromString(chat) }.isFailure) return null
        return Pair(chat, uri.host == "schedules")
    }
}
