package com.nexa.mobile

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.core.app.NotificationCompat

/** Chat-scoped delivery from either the visible executor or an Android job. */
internal object PhoneReplies {
    private const val APPROVAL = 73
    private const val OPEN = "com.nexa.mobile.OPEN_REPLY"
    private var viewedRun: String? = null

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

    fun clearApproval(context: Context, run: String) {
        context.getSystemService(NotificationManager::class.java).cancel(run, APPROVAL)
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
