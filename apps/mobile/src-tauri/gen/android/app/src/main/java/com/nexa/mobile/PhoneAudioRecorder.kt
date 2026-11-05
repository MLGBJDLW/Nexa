package com.nexa.mobile

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import android.util.Base64
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Bounded foreground recording for the user's configured transcription API. */
class PhoneAudioRecorder {
    companion object {
        const val SAMPLE_RATE = 16_000
        const val MAX_BYTES = SAMPLE_RATE * 2 * 60
        fun wav(pcm: ByteArray): ByteArray {
            require(pcm.size <= MAX_BYTES && pcm.size % 2 == 0)
            return ByteBuffer.allocate(44 + pcm.size).order(ByteOrder.LITTLE_ENDIAN).apply {
                put("RIFF".toByteArray()); putInt(36 + pcm.size); put("WAVEfmt ".toByteArray())
                putInt(16); putShort(1); putShort(1); putInt(SAMPLE_RATE); putInt(SAMPLE_RATE * 2)
                putShort(2); putShort(16); put("data".toByteArray()); putInt(pcm.size); put(pcm)
            }.array()
        }
    }
    private data class Session(val invoke: Invoke, val recorder: AudioRecord, val stop: AtomicBoolean = AtomicBoolean(false))
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private var session: Session? = null
    val active get() = session != null

    fun start(invoke: Invoke) {
        if (session != null) { invoke.reject("speech_busy"); return }
        var recorder: AudioRecord? = null
        try {
            val minimum = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            check(minimum > 0)
            recorder = AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minimum * 2, 8192))
            check(recorder.state == AudioRecord.STATE_INITIALIZED)
            recorder.startRecording()
            check(recorder.recordingState == AudioRecord.RECORDSTATE_RECORDING)
            val current = Session(invoke, recorder); session = current
            worker.execute {
                var audio: ByteArray? = null; var failure: String? = null
                try {
                    val bytes = ByteArrayOutputStream()
                    val buffer = ByteArray(4096)
                    while (!current.stop.get() && bytes.size() < MAX_BYTES) {
                        val count = current.recorder.read(buffer, 0, minOf(buffer.size, MAX_BYTES - bytes.size()))
                        if (count > 0) bytes.write(buffer, 0, count)
                        else if (!current.stop.get()) error("Microphone read failed")
                    }
                    if (bytes.size() < SAMPLE_RATE / 5) failure = "speech_no_match"
                    else audio = wav(bytes.toByteArray())
                } catch (_: Exception) { failure = "speech_failed" }
                finally {
                    try { current.recorder.stop() } catch (_: Exception) { }
                    current.recorder.release()
                }
                main.post {
                    if (session !== current) return@post
                    session = null
                    if (failure != null) current.invoke.reject(failure)
                    else current.invoke.resolve(JSObject().apply { put("audioBase64", Base64.encodeToString(audio, Base64.NO_WRAP)); put("mimeType", "audio/wav") })
                }
            }
        } catch (_: SecurityException) { recorder?.release(); session = null; invoke.reject("speech_permission") }
        catch (_: Exception) { recorder?.release(); session = null; invoke.reject("speech_failed") }
    }
    fun finish() { session?.let { it.stop.set(true); try { it.recorder.stop() } catch (_: Exception) { } } }
    fun cancel(reason: String = "speech_cancelled") {
        val current = session ?: return
        session = null; current.stop.set(true)
        try { current.recorder.stop() } catch (_: Exception) { }
        current.invoke.reject(reason)
    }
    fun close() { cancel("speech_interrupted"); worker.shutdown(); main.removeCallbacksAndMessages(null) }
}
