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
        for (command in listOf("settings put secure enabled_accessibility_services com.nexa.mobile/.PhoneAccessibilityService", "settings put secure accessibility_enabled 1")) {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command)).use { it.readBytes() }
        }
        val previous = PhoneControlFixtureActivity.current
        instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, PhoneControlFixtureActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
        val deadline = SystemClock.elapsedRealtime() + 10_000
        while ((PhoneAccessibilityService.instance == null || PhoneControlFixtureActivity.current == null || PhoneControlFixtureActivity.current === previous || PhoneControlFixtureActivity.current?.hasWindowFocus() != true) && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(50)
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
