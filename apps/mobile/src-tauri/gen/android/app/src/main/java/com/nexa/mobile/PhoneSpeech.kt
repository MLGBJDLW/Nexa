package com.nexa.mobile

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Channel
import app.tauri.annotation.InvokeArg
import java.util.Locale
import java.util.UUID

@InvokeArg
class DictationArgs {
    var progress: Channel? = null
    var requestId: String? = null
}

/** Only the active request receives acoustic levels and cumulative interim words. */
private class DictationProgress(private val args: DictationArgs) {
    private var lastLevelAt = 0L
    fun level(value: Float) {
        val now = SystemClock.elapsedRealtime()
        if (!value.isFinite() || now - lastLevelAt < 75) return
        lastLevelAt = now
        send("level", value.coerceIn(0f, 1f))
    }
    fun send(key: String, value: Any) {
        args.progress?.send(JSObject().apply { put("requestId", args.requestId); put(key, value) })
    }
}

/** One activity-owned speech session. No WebView microphone or embedded model. */
class PhoneSpeech(private val activity: Activity) {
    private val main = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    private var listening: Invoke? = null
    private var tts: TextToSpeech? = null
    private var ready = false
    private var speaking: Invoke? = null
    private var speechId = ""
    private var engineGeneration = 0L
    private var initTimeout: Runnable? = null
    private val preferences = activity.getSharedPreferences("phone-speech", Activity.MODE_PRIVATE)
    private val recorder = PhoneAudioRecorder()
    private val audioPlayer = PhoneAudioPlayer()
    val inputBusy: Boolean get() = listening != null || recorder.active
    private fun onDeviceAvailable() = Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(activity)
    fun status() = JSObject().apply {
        put("mode", preferences.getString("mode", "system"))
        put("outputMode", preferences.getString("outputMode", "system"))
        put("language", preferences.getString("language", ""))
        put("systemAvailable", SpeechRecognizer.isRecognitionAvailable(activity))
        put("onDeviceAvailable", onDeviceAvailable())
        put("microphoneGranted", androidx.core.content.ContextCompat.checkSelfPermission(activity, android.Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED)
    }
    fun configure(args: JSObject): JSObject {
        val mode = args.getString("mode", "system").orEmpty()
        val language = args.getString("language", "").orEmpty()
        val outputMode = args.getString("outputMode", preferences.getString("outputMode", "system")).orEmpty()
        require(outputMode in listOf("system", "cloud")) { "Unsupported speech output mode." }
        require(mode in listOf("system", "on_device", "cloud")) { "Unsupported speech mode." }
        require(language.isEmpty() || language.matches(Regex("[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*"))) { "Choose a valid speech language." }
        preferences.edit().putString("mode", mode).putString("outputMode", outputMode).putString("language", language).apply()
        return status()
    }
    private fun language(args: JSObject): String = preferences.getString("language", "").orEmpty().ifBlank {
        args.getString("language", "").orEmpty().ifBlank { Locale.getDefault().toLanguageTag() }
    }
    private val recognitionTimeout = Runnable { finishRecognition(error = "speech_timeout") }
    private fun resetEngine() {
        engineGeneration++
        initTimeout?.let { main.removeCallbacks(it) }; initTimeout = null
        tts?.shutdown(); tts = null; ready = false
    }

    fun listen(invoke: Invoke) = activity.runOnUiThread {
        if (listening != null || recorder.active) { invoke.reject("speech_busy"); return@runOnUiThread }
        val progress = if (invoke.getArgs().has("progress")) DictationProgress(invoke.parseArgs(DictationArgs::class.java)) else null
        if (preferences.getString("mode", "system") == "cloud") {
            stopSpeech()
            recorder.start(invoke, progress?.let { it::level }) { progress?.send("captured", true) }
            return@runOnUiThread
        }
        val requestedOnDevice = preferences.getString("mode", "system") == "on_device"
        if (requestedOnDevice && !onDeviceAvailable()) { invoke.reject("speech_on_device_unavailable"); return@runOnUiThread }
        val onDevice = requestedOnDevice || (!SpeechRecognizer.isRecognitionAvailable(activity) && onDeviceAvailable())
        if (!onDevice && !SpeechRecognizer.isRecognitionAvailable(activity)) { invoke.reject("speech_unavailable"); return@runOnUiThread }
        stopSpeech()
        listening = invoke
        startRecognition(invoke, onDevice, progress)
    }
    private fun startRecognition(invoke: Invoke, onDevice: Boolean, progress: DictationProgress?) {
        try {
            val session = if (onDevice && Build.VERSION.SDK_INT >= 31) SpeechRecognizer.createOnDeviceSpeechRecognizer(activity) else SpeechRecognizer.createSpeechRecognizer(activity)
            recognizer = session
            var partialText = ""
            fun current() = listening === invoke && recognizer === session
            session.setRecognitionListener(object : RecognitionListener {
                override fun onReadyForSpeech(params: Bundle?) { if (current()) armRecognitionTimeout() }
                override fun onBeginningOfSpeech() { if (current()) armRecognitionTimeout() }
                override fun onRmsChanged(rmsdB: Float) { if (current()) progress?.level(rmsdB / 12f) }
                override fun onBufferReceived(buffer: ByteArray?) {
                    if (current() && buffer != null) progress?.level(PhoneAudioRecorder.pcmLevel(buffer, buffer.size, bigEndian = true))
                }
                override fun onEndOfSpeech() { if (current()) armRecognitionTimeout() }
                override fun onError(error: Int) {
                    if (!current()) return
                    // A broken/default network service must not make an
                    // available Android on-device engine unreachable. Retry
                    // once locally, never loop or silently enable a cloud API.
                    if (!onDevice && partialText.isBlank() && onDeviceAvailable() && error in listOf(SpeechRecognizer.ERROR_CLIENT, SpeechRecognizer.ERROR_SERVER, SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT)) {
                        recognizer = null; session.cancel(); session.destroy()
                        startRecognition(invoke, true, progress); return
                    }
                    val message = when (error) {
                        SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "speech_no_match"
                        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "speech_permission"
                        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "speech_network"
                        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "speech_busy"
                        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED, SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE -> "speech_language"
                        else -> "speech_failed"
                    }
                    finishRecognition(error = message)
                }
                override fun onResults(results: Bundle?) {
                    if (current()) finishRecognition(results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull { it.isNotBlank() } ?: partialText)
                }
                override fun onPartialResults(partialResults: Bundle?) { if (current()) {
                    partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull { it.isNotBlank() }?.takeIf { it != partialText }?.let {
                        partialText = it
                        progress?.send("text", it)
                    }
                    armRecognitionTimeout()
                } }
                override fun onEvent(eventType: Int, params: Bundle?) {}
            })
            session.startListening(Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE, language(invoke.getArgs()))
                .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true))
            armRecognitionTimeout()
        } catch (error: Exception) { finishRecognition(error = "speech_failed") }
    }
    private fun armRecognitionTimeout() { main.removeCallbacks(recognitionTimeout); main.postDelayed(recognitionTimeout, 15_000) }
    private fun finishRecognition(text: String = "", error: String? = null) {
        val pending = listening; listening = null
        main.removeCallbacks(recognitionTimeout)
        recognizer?.cancel(); recognizer?.destroy(); recognizer = null
        val failure = error ?: if (text.isBlank()) "speech_no_match" else null
        if (failure == null) pending?.resolve(JSObject().apply { put("text", text) }) else pending?.reject(failure)
    }
    fun finishListening() = activity.runOnUiThread { if (recorder.active) recorder.finish() else recognizer?.stopListening() }
    fun cancelListening() = activity.runOnUiThread { recorder.cancel(); finishRecognition(error = "speech_cancelled") }

    fun speak(invoke: Invoke) = activity.runOnUiThread {
        recorder.cancel()
        finishRecognition(error = "speech_cancelled")
        stopSpeech()
        speaking = invoke
        val requestId = UUID.randomUUID().toString(); speechId = requestId
        if (ready) play(invoke, requestId)
        else if (tts == null) {
            val generation = ++engineGeneration
            val timeout = Runnable {
                if (generation == engineGeneration) { finishSpeech("speech_timeout"); resetEngine() }
            }
            initTimeout = timeout; main.postDelayed(timeout, 15_000)
            tts = TextToSpeech(activity) { result -> main.post {
                if (generation != engineGeneration) return@post
                main.removeCallbacks(timeout); initTimeout = null
                ready = result == TextToSpeech.SUCCESS
                if (!ready) { finishSpeech("speech_unavailable"); resetEngine() }
                else speaking?.let { play(it, speechId) }
            } }
        }
    }
    private fun play(invoke: Invoke, requestId: String) {
        val engine = tts ?: run { ready = false; finishSpeech("speech_unavailable"); return }
        val text = invoke.getArgs().getString("text", "").orEmpty()
        val locale = Locale.forLanguageTag(language(invoke.getArgs()))
        if (engine.setLanguage(locale) < TextToSpeech.LANG_AVAILABLE) { finishSpeech("speech_language"); return }
        val size = minOf(3000, TextToSpeech.getMaxSpeechInputLength() - 1)
        // Android's limit counts UTF-16 units; keep surrogate pairs together.
        val chunks = mutableListOf<String>(); var start = 0
        while (start < text.length) {
            var end = minOf(text.length, start + size)
            if (end < text.length && text[end - 1].isHighSurrogate()) end--
            chunks.add(text.substring(start, end)); start = end
        }
        if (chunks.isEmpty()) { finishSpeech(); return }
        engine.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) {}
            override fun onDone(utteranceId: String?) { if (utteranceId == "$requestId:${chunks.lastIndex}") main.post { if (speechId == requestId) finishSpeech() } }
            @Deprecated("Android legacy callback") override fun onError(utteranceId: String?) { main.post { if (speechId == requestId) { engine.stop(); finishSpeech("speech_failed") } } }
        })
        for ((index, chunk) in chunks.withIndex()) {
            if (engine.speak(chunk, TextToSpeech.QUEUE_ADD, null, "$requestId:$index") == TextToSpeech.ERROR) { engine.stop(); finishSpeech("speech_failed"); break }
        }
    }
    private fun finishSpeech(error: String? = null) {
        val pending = speaking; speaking = null; speechId = ""
        if (error == null) pending?.resolve(JSObject()) else pending?.reject(error)
    }
    fun play(invoke: Invoke) = activity.runOnUiThread { cancelListening(); stopSpeech(); audioPlayer.play(invoke) }
    fun stopSpeech() { audioPlayer.stop(); tts?.stop(); finishSpeech() }
    fun stop() = activity.runOnUiThread { recorder.cancel("speech_interrupted"); audioPlayer.stop("speech_interrupted"); finishRecognition(error = "speech_interrupted"); stopSpeech() }
    fun destroy() = activity.runOnUiThread { stop(); recorder.close(); resetEngine(); main.removeCallbacksAndMessages(null) }
}
