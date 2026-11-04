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
    private val recognitionTimeout = Runnable { finishRecognition(error = "Speech recognition stopped responding. Try again or use the keyboard.") }
    private val initTimeout = Runnable { finishSpeech("The speech engine did not initialize. Check Android text-to-speech settings."); tts?.shutdown(); tts = null; ready = false }

    fun listen(invoke: Invoke) = activity.runOnUiThread {
        if (listening != null) { invoke.reject("A dictation is already active."); return@runOnUiThread }
        if (!SpeechRecognizer.isRecognitionAvailable(activity)) { invoke.reject("No Android speech recognition service is installed. Choose one in system voice input settings, or use the keyboard."); return@runOnUiThread }
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
                        SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "No speech recognized. Try again."
                        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Allow microphone access in Android app settings."
                        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "The speech service could not connect. Check the network or select an offline speech engine."
                        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "The speech service is busy. Close other microphone apps and try again."
                        else -> "Android speech recognition failed (code $error). Check the installed voice input service."
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
        } catch (error: Exception) { finishRecognition(error = error.message ?: "Speech input is unavailable.") }
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
            main.postDelayed(initTimeout, 15_000)
            tts = TextToSpeech(activity) { result -> main.post {
                main.removeCallbacks(initTimeout)
                ready = result == TextToSpeech.SUCCESS
                if (!ready) { finishSpeech("Android text-to-speech is unavailable. Install or enable a speech engine."); tts?.shutdown(); tts = null }
                else speaking?.let { play(it, speechId) }
            } }
        }
    }
    private fun play(invoke: Invoke, requestId: String) {
        val engine = tts ?: return
        val text = invoke.getArgs().getString("text", "").orEmpty()
        val locale = Locale.forLanguageTag(invoke.getArgs().getString("language", Locale.getDefault().toLanguageTag()))
        if (engine.setLanguage(locale) < TextToSpeech.LANG_AVAILABLE) { finishSpeech("This voice language is unavailable. Install its voice data in Android text-to-speech settings."); return }
        val size = minOf(3000, TextToSpeech.getMaxSpeechInputLength() - 1)
        // Chunk by Unicode code points so supplementary characters stay intact.
        val points = text.codePoints().toArray()
        val chunks = points.toList().chunked(size).map { String(it.toIntArray(), 0, it.size) }
        if (chunks.isEmpty()) { finishSpeech(); return }
        engine.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) {}
            override fun onDone(utteranceId: String?) { if (utteranceId == "$requestId:${chunks.lastIndex}") main.post { if (speechId == requestId) finishSpeech() } }
            @Deprecated("Android legacy callback") override fun onError(utteranceId: String?) { main.post { if (speechId == requestId) finishSpeech("The Android speech engine could not read this message.") } }
        })
        for ((index, chunk) in chunks.withIndex()) {
            if (engine.speak(chunk, TextToSpeech.QUEUE_ADD, null, "$requestId:$index") == TextToSpeech.ERROR) { engine.stop(); finishSpeech("The Android speech engine could not start playback."); break }
        }
    }
    private fun finishSpeech(error: String? = null) {
        val pending = speaking; speaking = null; speechId = ""
        if (error == null) pending?.resolve(JSObject()) else pending?.reject(error)
    }
    fun stopSpeech() { tts?.stop(); finishSpeech() }
    fun stop() = activity.runOnUiThread { finishRecognition(); stopSpeech() }
    fun destroy() = activity.runOnUiThread { stop(); main.removeCallbacksAndMessages(null); tts?.shutdown(); tts = null; ready = false }
}
