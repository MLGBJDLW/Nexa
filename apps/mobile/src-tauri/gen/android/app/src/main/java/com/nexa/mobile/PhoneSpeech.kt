package com.nexa.mobile

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.Locale
import java.util.UUID

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
    private val recognitionTimeout = Runnable { finishRecognition(error = "speech_timeout") }
    private fun resetEngine() {
        engineGeneration++
        initTimeout?.let { main.removeCallbacks(it) }; initTimeout = null
        tts?.shutdown(); tts = null; ready = false
    }

    fun listen(invoke: Invoke) = activity.runOnUiThread {
        if (listening != null) { invoke.reject("speech_busy"); return@runOnUiThread }
        if (!SpeechRecognizer.isRecognitionAvailable(activity)) { invoke.reject("speech_unavailable"); return@runOnUiThread }
        try {
            stopSpeech()
            listening = invoke
            val session = SpeechRecognizer.createSpeechRecognizer(activity)
            recognizer = session
            session.setRecognitionListener(object : RecognitionListener {
                override fun onReadyForSpeech(params: Bundle?) { if (listening === invoke) armRecognitionTimeout() }
                override fun onBeginningOfSpeech() { if (listening === invoke) armRecognitionTimeout() }
                override fun onRmsChanged(rmsdB: Float) { if (listening === invoke) armRecognitionTimeout() }
                override fun onBufferReceived(buffer: ByteArray?) {}
                override fun onEndOfSpeech() { if (listening === invoke) armRecognitionTimeout() }
                override fun onError(error: Int) {
                    if (listening !== invoke) return
                    val message = when (error) {
                        SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "speech_no_match"
                        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "speech_permission"
                        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "speech_network"
                        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "speech_busy"
                        else -> "speech_failed"
                    }
                    finishRecognition(error = message)
                }
                override fun onResults(results: Bundle?) {
                    if (listening === invoke) finishRecognition(results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty())
                }
                override fun onPartialResults(partialResults: Bundle?) { if (listening === invoke) armRecognitionTimeout() }
                override fun onEvent(eventType: Int, params: Bundle?) {}
            })
            val language = invoke.getArgs().getString("language", Locale.getDefault().toLanguageTag())
            session.startListening(Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
                .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true))
            armRecognitionTimeout()
        } catch (error: Exception) { finishRecognition(error = "speech_failed") }
    }
    private fun armRecognitionTimeout() { main.removeCallbacks(recognitionTimeout); main.postDelayed(recognitionTimeout, 15_000) }
    private fun finishRecognition(text: String = "", error: String? = null) {
        val pending = listening; listening = null
        main.removeCallbacks(recognitionTimeout)
        recognizer?.cancel(); recognizer?.destroy(); recognizer = null
        if (error == null) pending?.resolve(JSObject().apply { put("text", text) }) else pending?.reject(error)
    }
    fun cancelListening() = activity.runOnUiThread { finishRecognition() }

    fun speak(invoke: Invoke) = activity.runOnUiThread {
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
        val locale = Locale.forLanguageTag(invoke.getArgs().getString("language", Locale.getDefault().toLanguageTag()))
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
            @Deprecated("Android legacy callback") override fun onError(utteranceId: String?) { main.post { if (speechId == requestId) finishSpeech("speech_failed") } }
        })
        for ((index, chunk) in chunks.withIndex()) {
            if (engine.speak(chunk, TextToSpeech.QUEUE_ADD, null, "$requestId:$index") == TextToSpeech.ERROR) { engine.stop(); finishSpeech("speech_failed"); break }
        }
    }
    private fun finishSpeech(error: String? = null) {
        val pending = speaking; speaking = null; speechId = ""
        if (error == null) pending?.resolve(JSObject()) else pending?.reject(error)
    }
    fun stopSpeech() { tts?.stop(); finishSpeech() }
    fun stop() = activity.runOnUiThread { finishRecognition(); stopSpeech() }
    fun destroy() = activity.runOnUiThread { stop(); resetEngine(); main.removeCallbacksAndMessages(null) }
}
