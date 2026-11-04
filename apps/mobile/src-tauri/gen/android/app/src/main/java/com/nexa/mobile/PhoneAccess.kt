package com.nexa.mobile

import android.app.Activity
import android.app.ActivityManager
import android.app.NotificationManager
import android.app.KeyguardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import app.tauri.plugin.JSObject

/** Report observed state; Android does not expose a reliable restricted-settings flag. */
object PhoneAccess {
    fun status(context: Context): JSObject {
        val component = ComponentName(context, PhoneAccessibilityService::class.java)
        val enabled = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
            ?.split(':')?.any { ComponentName.unflattenFromString(it) == component } == true
        val connected = PhoneAccessibilityService.instance != null
        return JSObject().apply {
            put("platform", "android")
            put("manufacturer", Build.MANUFACTURER)
            put("androidVersion", Build.VERSION.SDK_INT)
            put("accessibility", connected)
            put("accessibilityEnabled", enabled)
            put("accessibilityStatus", if (connected) "connected" else if (enabled) "disconnected" else "disabled")
            put("restrictedSettingsMayApply", Build.VERSION.SDK_INT >= 33 && !enabled)
            put("batteryOptimized", !context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName))
            put("backgroundRestricted", context.getSystemService(ActivityManager::class.java).isBackgroundRestricted)
            put("notifications", context.getSystemService(NotificationManager::class.java).areNotificationsEnabled())
            put("screenCapture", connected && Build.VERSION.SDK_INT >= 30)
            put("screenInteractive", context.getSystemService(PowerManager::class.java).isInteractive)
            put("screenLocked", context.getSystemService(KeyguardManager::class.java).isKeyguardLocked)
            put("assistantOverlay", context.getSharedPreferences("phone-assistant", Context.MODE_PRIVATE).getBoolean("overlay", false))
            put("phoneFiles", true)
            put("localAgent", true)
        }
    }

    fun screenRecovery(context: Context): JSObject? {
        val state = status(context)
        if (state.getBoolean("screenInteractive") && !state.getBoolean("screenLocked")) return null
        return JSObject().apply {
            put("code", "screen_locked_or_asleep"); put("requiresUserAction", true); put("retryable", false)
            put("message", "Wake and unlock the phone to continue phone control. Accessibility is still connected; no new permission is needed.")
            put("recovery", "Ask the user to wake/unlock the phone, then observe again. Never bypass the lock screen.")
        }
    }

    fun recovery(context: Context): JSObject {
        val state = status(context)
        val disconnected = state.getBoolean("accessibilityEnabled")
        return JSObject().apply {
            put("code", if (disconnected) "accessibility_disconnected" else "accessibility_disabled")
            put("requiresUserAction", true)
            put("retryable", false)
            put("capabilities", state)
            put("message", if (disconnected)
                "Nexa accessibility is enabled but its service is disconnected. Open Phone settings > Phone control access, turn Nexa off and on, then return. If this recurs, review the app's battery/background restrictions."
                else "Enable Nexa in Phone settings > Phone control access. If Android shows Restricted settings, open App info > More > Allow restricted settings, then return to Accessibility and enable Nexa. The app cannot grant this system permission itself.")
            put("recovery", "Ask the user to complete the relevant step in Phone settings. Retry only after status reports accessibility=true.")
        }
    }

    fun open(activity: Activity, destination: String) {
        val appInfo = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${activity.packageName}"))
        val intent = when (destination) {
            "app_info" -> appInfo
            "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            "accessibility" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
            "notifications" -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
            else -> error("Unsupported settings destination.")
        }
        try { activity.startActivity(intent) }
        catch (_: android.content.ActivityNotFoundException) { activity.startActivity(appInfo) }
    }
}
