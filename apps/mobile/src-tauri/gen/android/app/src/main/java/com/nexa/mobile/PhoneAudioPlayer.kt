package com.nexa.mobile

import android.media.AudioAttributes
import android.media.MediaDataSource
import android.media.MediaPlayer
import android.util.Base64
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

/** Native playback, with no audio, credentials or temporary files in the WebView. */
class PhoneAudioPlayer {
    private var player: MediaPlayer? = null
    private var pending: Invoke? = null
    fun play(invoke: Invoke) {
        stop()
        try {
            val encoded = invoke.getArgs().getString("audioBase64", "").orEmpty()
            require(encoded.length <= 12 * 1024 * 1024)
            val bytes = Base64.decode(encoded, Base64.DEFAULT)
            require(bytes.isNotEmpty() && bytes.size <= 8 * 1024 * 1024)
            val session = MediaPlayer(); player = session; pending = invoke
            session.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANCE_ACCESSIBILITY).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            session.setDataSource(object : MediaDataSource() {
                override fun getSize() = bytes.size.toLong()
                override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
                    if (position < 0 || position >= bytes.size) return -1
                    val count = minOf(size, bytes.size - position.toInt())
                    bytes.copyInto(buffer, offset, position.toInt(), position.toInt() + count); return count
                }
                override fun close() { bytes.fill(0) }
            })
            session.setOnPreparedListener { if (player === session) session.start() }
            session.setOnCompletionListener { if (player === session) stop() }
            session.setOnErrorListener { _, _, _ -> if (player === session) stop("speech_failed"); true }
            session.prepareAsync()
        } catch (_: Exception) {
            if (pending === invoke) stop("speech_failed") else invoke.reject("speech_failed")
        }
    }
    fun stop(error: String? = null) {
        val invoke = pending; pending = null
        val previous = player; player = null; previous?.release()
        if (error == null) invoke?.resolve(JSObject()) else invoke?.reject(error)
    }
}
