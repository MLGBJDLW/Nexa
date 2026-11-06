package com.nexa.mobile

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.util.Log
import org.json.JSONObject
import org.json.JSONTokener
import java.time.Instant
import java.util.UUID
import java.util.concurrent.Executors

/** The OS owns one pending wake. SQLite owns cron, approvals, retries and execution. */
internal object PhoneScheduleJobs {
    private const val FIRST = 7310
    private const val SECOND = 7311
    private const val DUE = "nexa.schedule.due"
    private var runningId: Int? = null

    fun started(id: Int) { runningId = id }
    fun stopped(id: Int) { if (runningId == id) runningId = null }

    // All changes run on Android's main looper. Use the alternate slot when a
    // job is running: replacing its own JobInfo would interrupt a live turn.
    fun reconcile(context: Context, nextWakeAt: String?, timeChanged: Boolean = false) {
        val scheduler = context.getSystemService(JobScheduler::class.java)
        val waiting = listOf(FIRST, SECOND).filter { it != runningId }.mapNotNull(scheduler::getPendingJob)
        if (nextWakeAt == null) {
            waiting.forEach { scheduler.cancel(it.id) }
            return
        }
        val due = Instant.parse(nextWakeAt).toEpochMilli()
        if (!timeChanged && waiting.any { it.extras.getLong(DUE) == due }) return
        val id = if (runningId == FIRST) SECOND else FIRST
        val delay = (due - System.currentTimeMillis()).coerceAtLeast(10_000)
        val job = JobInfo.Builder(id, ComponentName(context, PhoneScheduleService::class.java))
            .setExtras(PersistableBundle().apply { putLong(DUE, due) })
            .setPersisted(true)
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            .setMinimumLatency(delay)
            .setBackoffCriteria(30_000, JobInfo.BACKOFF_POLICY_EXPONENTIAL)
            .build()
        check(scheduler.schedule(job) == JobScheduler.RESULT_SUCCESS) { "Android could not schedule the next phone job." }
        waiting.filter { it.id != id }.forEach { scheduler.cancel(it.id) }
    }

    fun clockChanged(context: Context) {
        val jobs = context.getSystemService(JobScheduler::class.java)
        val due = listOf(FIRST, SECOND).filter { it != runningId }.mapNotNull(jobs::getPendingJob)
            .minOfOrNull { it.extras.getLong(DUE) } ?: return
        reconcile(context, Instant.ofEpochMilli(due).toString(), timeChanged = true)
    }
}

class PhoneScheduleClockReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_TIME_CHANGED) PhoneScheduleJobs.clockChanged(context)
    }
}

internal object PhoneScheduleBridge {
    init { System.loadLibrary("nexa_mobile_lib") }
    @JvmStatic external fun tick(appDataDir: String, settingsKey: String, connectionJson: String, workerId: String): String
    @JvmStatic external fun stop(workerId: String)
}

class PhoneScheduleService : JobService() {
    companion object {
        private val executor = Executors.newSingleThreadExecutor()
    }
    private class Attempt(val id: String = UUID.randomUUID().toString()) {
        var stopped = false
        var inNative = false
    }
    private val attempts = mutableMapOf<Int, Attempt>()
    private val main = Handler(Looper.getMainLooper())

    override fun onStartJob(params: JobParameters): Boolean {
        val attempt = Attempt()
        attempts[params.jobId] = attempt
        PhoneScheduleJobs.started(params.jobId)
        executor.execute {
            val result = try {
                val secrets = SecretStore(applicationContext)
                val key = secrets.read("settings-key")?.let { JSONTokener(it).nextValue() as? String }
                val store = secrets.read("local-provider")?.let(::JSONObject)
                val connection = store?.optJSONObject("connections")?.optJSONObject(store.optString("activeId"))
                // Load the native library off the service thread even on cold start.
                val bridge = PhoneScheduleBridge
                synchronized(attempt) {
                    if (attempt.stopped) return@execute
                    attempt.inNative = true
                }
                JSONObject(bridge.tick(dataDir.absolutePath, key.orEmpty(), connection?.toString().orEmpty(), attempt.id))
            } catch (e: Exception) {
                Log.w("NexaSchedules", "Background schedule check failed (${e.javaClass.simpleName})")
                null
            } catch (e: LinkageError) {
                Log.e("NexaSchedules", "Background schedule runtime unavailable (${e.javaClass.simpleName})")
                null
            } finally {
                synchronized(attempt) { attempt.inNative = false }
            }
            main.post {
                if (attempts[params.jobId] !== attempt) return@post
                attempts.remove(params.jobId)
                var retry = result == null || !result.has("nextWakeAt")
                try {
                    if (!retry && result != null) {
                        val next = if (result.isNull("nextWakeAt")) null else result.getString("nextWakeAt")
                        if (next == null && result.optString("status") == "error") retry = true
                        else PhoneScheduleJobs.reconcile(this, next)
                        val chat = result.optString("conversationId")
                        val run = result.optString("runId")
                        if (chat.isNotEmpty() && run.isNotEmpty() && result.optString("status") in setOf("completed", "waiting_approval")) {
                            PhoneReplies.post(this, chat, run, result.optString("status") == "waiting_approval")
                        }
                    }
                } catch (e: Exception) {
                    Log.w("NexaSchedules", "Could not rearm background schedule (${e.javaClass.simpleName})")
                    retry = true
                } finally {
                    PhoneScheduleJobs.stopped(params.jobId)
                    jobFinished(params, retry)
                }
            }
        }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean {
        val attempt = attempts.remove(params.jobId) ?: return false
        synchronized(attempt) {
            attempt.stopped = true
            if (attempt.inNative) PhoneScheduleBridge.stop(attempt.id)
        }
        PhoneScheduleJobs.stopped(params.jobId)
        return true
    }
}
