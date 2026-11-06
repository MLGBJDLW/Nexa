package com.nexa.mobile

import android.content.Intent
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import com.fasterxml.jackson.databind.ObjectMapper
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class PhoneControlInstrumentedTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private lateinit var service: PhoneAccessibilityService
    @Test fun floatingAssistantAnimatesAndReleasesItsWindow() {
        val automation = instrumentation.getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        automation.serviceInfo = automation.serviceInfo.apply { flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS }
        fun durationScale(value: Int) { android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand("settings put global animator_duration_scale $value")).use { it.readBytes() } }
        fun capture(name: String): android.graphics.Bitmap {
            val bitmap = requireNotNull(automation.takeScreenshot())
            java.io.File(instrumentation.targetContext.getExternalFilesDir(null), name).outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            return bitmap
        }
        lateinit var overlay: PhoneAssistantOverlay
        durationScale(1)
        try {
            instrumentation.runOnMainSync { service.setOverlay(false); overlay = PhoneAssistantOverlay(service); overlay.show() }
            SystemClock.sleep(200)
            val bounds = android.graphics.Rect()
            val window = automation.windows.first { it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_ACCESSIBILITY_OVERLAY }
            window.getBoundsInScreen(bounds)
            fun pixels(bitmap: android.graphics.Bitmap): IntArray { val result = IntArray(bounds.width()*bounds.height()); bitmap.getPixels(result,0,bounds.width(),bounds.left,bounds.top,bounds.width(),bounds.height()); bitmap.recycle(); return result }
            val first = pixels(capture("overlay-floating-first.png"))
            SystemClock.sleep(1100)
            val floating = pixels(capture("overlay-floating-second.png"))
            assertTrue("Floating surface should visibly move while idle", first.indices.count { first[it] != floating[it] } > 50)
            instrumentation.runOnMainSync { overlay.suspend() }
            SystemClock.sleep(100)
            val hidden = pixels(capture("overlay-suspended.png"))
            assertTrue("Suspended overlay must disappear during phone control", hidden.indices.count { hidden[it] != floating[it] } > 100)
            instrumentation.runOnMainSync { overlay.restore() }
            SystemClock.sleep(150)
            val restored = pixels(capture("overlay-restored.png"))
            assertTrue("Restoring resumes the visible surface", hidden.indices.count { hidden[it] != restored[it] } > 100)
            instrumentation.runOnMainSync { overlay.hide() }
            val deadline = SystemClock.elapsedRealtime() + 2000
            while (automation.windows.any { it.id == window.id } && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(40)
            assertFalse("The assistant window must detach on hide", automation.windows.any { it.id == window.id })
        } finally { instrumentation.runOnMainSync { runCatching { overlay.hide() } }; durationScale(0) }
    }

    @Test fun settingsEncryptionKeySurvivesStoreReopen() {
        val store = SecretStore(instrumentation.targetContext)
        val previous = store.read("settings-key")
        try {
            store.write("settings-key", "native-settings-key-fixture")
            assertEquals("native-settings-key-fixture", SecretStore(instrumentation.targetContext).read("settings-key"))
            val raw = instrumentation.targetContext.getSharedPreferences("nexa_secrets", android.content.Context.MODE_PRIVATE).getString("settings-key", "")
            assertFalse(raw.orEmpty().contains("native-settings-key-fixture"))
        } finally { store.write("settings-key", previous) }
    }
    @Before fun openFixture() {
        val automation = instrumentation.getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        for (command in listOf("settings put global window_animation_scale 0", "settings put global transition_animation_scale 0", "settings put global animator_duration_scale 0", "settings put secure enabled_accessibility_services com.nexa.mobile/.PhoneAccessibilityService", "settings put secure accessibility_enabled 1")) {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command)).use { it.readBytes() }
        }
        val previous = PhoneControlFixtureActivity.current
        instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, PhoneControlFixtureActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
        val deadline = SystemClock.elapsedRealtime() + 10_000
        var ready = false
        do {
            instrumentation.runOnMainSync { ready = PhoneAccessibilityService.instance != null && PhoneControlFixtureActivity.current?.let { it !== previous && it.hasWindowFocus() } == true }
            if (!ready) SystemClock.sleep(50)
        } while (!ready && SystemClock.elapsedRealtime() < deadline)
        assertTrue("Phone control fixture did not receive focus", ready)
        service = requireNotNull(PhoneAccessibilityService.instance)
        TaskService.stopRequested.set(false)
        instrumentation.waitForIdleSync()
        automation.waitForIdle(300, 5_000)
    }
    private fun call(args: JSONObject): JSONObject {
        val done = CountDownLatch(1)
        var result = JSONObject()
        val invoke = Invoke(1, "phoneControl", 1, 2, { callback, data ->
            result = JSONObject(data).apply { if (callback == 2L) put("nativeError", true) }
            done.countDown()
        }, args.toString(), ObjectMapper())
        instrumentation.runOnMainSync { service.execute(invoke) }
        assertTrue("Native operation timed out", done.await(15, TimeUnit.SECONDS))
        return result
    }
    private fun node(state: JSONObject, description: String): JSONObject {
        val nodes = state.getJSONArray("nodes")
        return (0 until nodes.length()).map { nodes.getJSONObject(it) }.firstOrNull { it.optString("description").equals(description, true) || it.optString("text").equals(description, true) }
            ?: throw AssertionError("Missing $description in $nodes")
    }
    private fun args(state: JSONObject, action: String) = JSONObject().put("action", action).put("observationId", state.getString("observationId")).put("packageName", state.getString("packageName"))
    private fun summary(state: JSONObject) = JSONObject(state.toString()).apply { optJSONObject("image")?.remove("base64") }.toString()

    @Test fun cloudDictationCapturesBoundedWavAndCancellationDiscardsAudio() {
        val automation = instrumentation.getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand("pm grant com.nexa.mobile android.permission.RECORD_AUDIO")).use { it.readBytes() }
        lateinit var speech: PhoneSpeech
        val done = CountDownLatch(1); var result = JSONObject(); var rejected = false
        val invoke = Invoke(1, "speech", 1, 2, { callback, data -> rejected = callback == 2L; result = JSONObject(data); done.countDown() }, "{}", ObjectMapper())
        instrumentation.runOnMainSync {
            speech = PhoneSpeech(requireNotNull(PhoneControlFixtureActivity.current))
            speech.configure(JSObject().apply { put("mode", "cloud") })
            speech.listen(invoke)
        }
        try {
            SystemClock.sleep(650)
            instrumentation.runOnMainSync { speech.finishListening() }
            assertTrue(done.await(8, TimeUnit.SECONDS)); assertFalse(result.toString(), rejected)
            val wav = android.util.Base64.decode(result.getString("audioBase64"), android.util.Base64.DEFAULT)
            assertEquals("RIFF", String(wav.copyOfRange(0, 4)))
            assertEquals("WAVE", String(wav.copyOfRange(8, 12)))
            assertTrue(wav.size > 44 && wav.size <= PhoneAudioRecorder.MAX_BYTES + 44)
            assertEquals(wav.size - 44, java.nio.ByteBuffer.wrap(wav, 40, 4).order(java.nio.ByteOrder.LITTLE_ENDIAN).int)
            val cancelled = CountDownLatch(1); var cancelledResult = ""
            val second = Invoke(2, "speech", 3, 4, { callback, data -> assertEquals(4L, callback); cancelledResult = data; cancelled.countDown() }, "{}", ObjectMapper())
            instrumentation.runOnMainSync { speech.listen(second); speech.cancelListening() }
            assertTrue(cancelled.await(8, TimeUnit.SECONDS)); assertTrue(cancelledResult.contains("speech_cancelled")); assertFalse(cancelledResult.contains("audioBase64"))
        } finally { instrumentation.runOnMainSync { speech.configure(JSObject().apply { put("mode", "system"); put("outputMode", "system") }); speech.destroy() } }
    }

    @Test fun cloudReadAloudPlaysProviderWavWithoutAWebViewOrDiskFile() {
        val complete = CountDownLatch(1); var rejected = false; var result = ""
        val args = JSONObject().put("audioBase64", android.util.Base64.encodeToString(PhoneAudioRecorder.wav(ByteArray(8000)), android.util.Base64.NO_WRAP))
        val invoke = Invoke(1, "speech", 1, 2, { callback, data -> rejected = callback == 2L; result = data; complete.countDown() }, args.toString(), ObjectMapper())
        val player = PhoneAudioPlayer()
        try {
            instrumentation.runOnMainSync { player.play(invoke) }
            assertTrue("Native playback did not finish", complete.await(10, TimeUnit.SECONDS))
            assertFalse(result, rejected)
        } finally { instrumentation.runOnMainSync { player.stop() } }
    }

    @Test fun emptyRecognitionIsNotReportedAsSuccessfulDictation() {
        var callbackId = 0L
        var payload = ""
        instrumentation.runOnMainSync {
            val speech = PhoneSpeech(requireNotNull(PhoneControlFixtureActivity.current))
            try {
                val invoke = Invoke(1, "speech", 1, 2, { id, data -> callbackId = id; payload = data }, "{}", ObjectMapper())
                PhoneSpeech::class.java.getDeclaredField("listening").apply { isAccessible = true }.set(speech, invoke)
                PhoneSpeech::class.java.getDeclaredMethod("finishRecognition", String::class.java, String::class.java).apply { isAccessible = true }.invoke(speech, "", null)
                assertEquals("Empty recognition must explain why the microphone stopped: $payload", 2L, callbackId)
                assertTrue(payload, payload.contains("speech_no_match"))
            } finally { speech.destroy() }
        }
    }

    @Test fun recoveryNavigationDoesNotRequireAnUnreadableObservation() {
        // A null root invalidates the previous observation. Recovery navigation
        // must remain possible without authorizing a stale click or send.
        instrumentation.runOnMainSync {
            PhoneAccessibilityService::class.java.getDeclaredMethod("invalidate").apply { isAccessible = true }.invoke(service)
        }
        val home = call(JSONObject().put("action", "home"))
        assertTrue(summary(home), home.optBoolean("actionAccepted"))
        val opened = call(JSONObject().put("action", "open_app").put("packageName", "com.android.settings"))
        assertTrue(summary(opened), opened.optBoolean("actionAccepted"))
        val click = call(JSONObject().put("action", "click").put("nodeId", 0).put("packageName", "com.android.settings"))
        assertEquals(summary(click), "stale_observation", click.optString("code"))
        assertFalse(click.optBoolean("actionPerformed"))
    }

    @Test fun reverseControlStopsAndRejectsAnotherComputer() {
        val automation = instrumentation.getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand("pm grant com.nexa.mobile android.permission.POST_NOTIFICATIONS")).use { it.readBytes() }
        lateinit var plugin: PhonePlugin
        instrumentation.runOnMainSync { plugin = PhonePlugin(requireNotNull(PhoneControlFixtureActivity.current)) }
        fun remote(action: String, host: String): JSONObject {
            val done = CountDownLatch(1); var result = JSONObject()
            val args = JSONObject().put("action",action).put("hostId",host).put("requestId",java.util.UUID.randomUUID().toString()).put("arguments",JSONObject().put("action","observe"))
            val invoke = Invoke(1,"remoteControl",1,2,{ callback,data -> result=JSONObject(data).apply { if(callback==2L)put("nativeError",true) }; done.countDown() },args.toString(),ObjectMapper())
            instrumentation.runOnMainSync { plugin.remoteControl(invoke) }
            assertTrue(done.await(8,TimeUnit.SECONDS)); return result
        }
        try {
            val enabled = remote("enable","computer-a")
            assertFalse(enabled.toString(), enabled.optBoolean("nativeError"))
            assertTrue(remote("status","computer-a").optBoolean("enabled"))
            assertTrue(remote("enable","computer-b").optBoolean("nativeError"))
            assertTrue(remote("status","computer-a").optBoolean("enabled"))
            assertTrue(remote("execute","computer-b").optBoolean("nativeError"))
            assertEquals("com.nexa.mobile",remote("execute","computer-a").optString("packageName"))
            assertFalse(remote("disable","computer-a").optBoolean("nativeError"))
            assertFalse(remote("status","computer-a").optBoolean("enabled"))
            assertTrue(remote("execute","computer-a").optBoolean("nativeError"))
        } finally { remote("disable","computer-a") }
    }

    @Test fun speechPreferencesSurviveRecreationAndValidateNativeMode() {
        instrumentation.runOnMainSync {
            val activity = requireNotNull(PhoneControlFixtureActivity.current)
            val speech = PhoneSpeech(activity)
            try {
                speech.configure(JSObject().apply { put("mode","on_device"); put("language","zh-CN") })
                val restored = PhoneSpeech(activity)
                try {
                    assertEquals("on_device", restored.status().getString("mode"))
                    assertEquals("zh-CN", restored.status().getString("language"))
                    assertTrue(restored.status().has("systemAvailable"))
                    assertTrue(restored.status().has("onDeviceAvailable"))
                    try { restored.configure(JSObject().apply { put("mode","invalid") }); fail("Invalid engines must be rejected") } catch (_: IllegalArgumentException) {}
                    restored.configure(JSObject().apply { put("mode","system"); put("language","") })
                } finally { restored.destroy() }
            } finally { speech.destroy() }
        }
    }

    @Test fun delayedScreenshotTapRevalidatesPixelsAndReturnsNextObservation() {
        val screen = call(JSONObject().put("action", "screenshot"))
        assertFalse(summary(screen), screen.optBoolean("nativeError"))
        val target = node(screen, "Apply message").getJSONArray("bounds")
        val image = screen.getJSONObject("image")
        val x = (target.getDouble(0) + target.getDouble(2)) / 2 * image.getInt("width") / image.getInt("displayWidth")
        val y = (target.getDouble(1) + target.getDouble(3)) / 2 * image.getInt("height") / image.getInt("displayHeight")
        // Reproduce model/approval latency without a slow wall-clock test.
        instrumentation.runOnMainSync {
            val field = PhoneAccessibilityService::class.java.getDeclaredField("observedAt").apply { isAccessible = true }
            field.setLong(service, SystemClock.elapsedRealtime() - 20_000)
        }
        val tapped = call(args(screen, "tap").put("x", x).put("y", y))
        assertFalse(summary(tapped), tapped.optBoolean("nativeError"))
        assertTrue(summary(tapped), tapped.optBoolean("actionAccepted"))
        assertNotEquals(screen.getString("observationId"), tapped.optString("observationId"))
        assertEquals("Applied 1", node(tapped, "Applied 1").getString("text"))
    }

    @Test fun semanticUnicodeInputReturnsVerifiedTextAndReusableNextObservation() {
        val initial = call(JSONObject().put("action", "observe"))
        val text = "你好 Nexa 🐼\n第二行 — café"
        val result = call(args(initial, "set_text").put("nodeId", node(initial, "Semantic message").getInt("nodeId")).put("text", text))
        assertTrue(result.toString(), result.optBoolean("textMatched"))
        assertNotEquals(initial.getString("observationId"), result.optString("observationId"))
        assertEquals(text, node(result, "Semantic message").getString("text"))
        val clicked = call(args(result, "click").put("nodeId", node(result, "Apply message").getInt("nodeId")))
        assertTrue(clicked.toString(), clicked.optBoolean("actionAccepted"))
        assertEquals("Applied 1", node(clicked, "Applied 1").getString("text"))
    }

    @Test fun nativeEditorFallbackReplacesUnicodeTextAcrossSuccessiveActions() {
        val initial = call(JSONObject().put("action", "observe"))
        var state = call(args(initial, "click").put("nodeId", node(initial, "Native message").getInt("nodeId")))
        assertTrue(state.toString(), state.optBoolean("nativeTextInput"))
        for (text in listOf("原生输入 🐼 café", "替换内容\n第二行")) {
            state = call(args(state, "set_text").put("nodeId", node(state, "Native message").getInt("nodeId")).put("text", text))
            assertTrue(state.toString(), state.optBoolean("textMatched"))
            assertEquals("native_input_connection", state.optString("inputMethod"))
            assertEquals(text, node(state, "Native message").getString("text"))
        }
    }

    @Test fun focusChangeCannotRedirectNodeLessTextInput() {
        val initial = call(JSONObject().put("action", "observe"))
        val focused = call(args(initial,"click").put("nodeId",node(initial,"Semantic message").getInt("nodeId")))
        instrumentation.runOnMainSync { PhoneControlFixtureActivity.current!!.findViewById<android.widget.EditText>(1002).requestFocus() }
        val result = call(args(focused,"set_text").put("text","Must not reach another field"))
        assertEquals(summary(result),"stale_observation",result.optString("code"))
        assertFalse(result.optBoolean("actionPerformed"))
        instrumentation.runOnMainSync {
            assertEquals("",PhoneControlFixtureActivity.current!!.findViewById<android.widget.EditText>(1001).text.toString())
            assertEquals("",PhoneControlFixtureActivity.current!!.findViewById<android.widget.EditText>(1002).text.toString())
        }
    }

    @Test fun changedPixelsReturnFreshEvidenceWithoutPerformingTheOldTap() {
        val screen = call(JSONObject().put("action", "screenshot"))
        val target = node(screen, "Apply message").getJSONArray("bounds")
        val image = screen.getJSONObject("image")
        instrumentation.runOnMainSync { PhoneControlFixtureActivity.current!!.findViewById<android.widget.TextView>(1003).text = "Changed before tap" }
        val result = call(args(screen, "tap")
            .put("x", (target.getDouble(0) + target.getDouble(2)) / 2 * image.getInt("width") / image.getInt("displayWidth"))
            .put("y", (target.getDouble(1) + target.getDouble(3)) / 2 * image.getInt("height") / image.getInt("displayHeight")))
        assertEquals("stale_observation", result.optString("code"))
        assertFalse(result.optBoolean("actionPerformed"))
        assertNotEquals(screen.getString("observationId"), result.getString("observationId"))
        assertEquals("Changed before tap", node(result, "Changed before tap").getString("text"))
    }
}
