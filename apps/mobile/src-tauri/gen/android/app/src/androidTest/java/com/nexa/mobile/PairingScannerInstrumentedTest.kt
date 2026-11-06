package com.nexa.mobile

import android.app.UiAutomation
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.os.SystemClock
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.journeyapps.barcodescanner.ScanOptions
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PairingScannerInstrumentedTest {
    @Test fun decodeCropTracksPortraitAndLandscape() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val automation = instrumentation.uiAutomation
        val context = instrumentation.targetContext
        automation.grantRuntimePermission(context.packageName, android.Manifest.permission.CAMERA)
        val intent = ScanOptions().setCaptureActivity(PairingScannerActivity::class.java)
            .setOrientationLocked(false).setDesiredBarcodeFormats(ScanOptions.QR_CODE).createScanIntent(context)
        try {
            ActivityScenario.launch<PairingScannerActivity>(intent).use { scenario ->
                for ((rotation, orientation) in listOf(
                    UiAutomation.ROTATION_FREEZE_0 to Configuration.ORIENTATION_PORTRAIT,
                    UiAutomation.ROTATION_FREEZE_90 to Configuration.ORIENTATION_LANDSCAPE
                )) {
                    automation.setRotation(rotation)
                    var ready = false
                    val deadline = SystemClock.elapsedRealtime() + 10_000
                    do {
                        scenario.onActivity { activity ->
                            assertEquals(ActivityInfo.SCREEN_ORIENTATION_FULL_USER, activity.requestedOrientation)
                            val preview = activity.findViewById<DecoratedBarcodeView>(R.id.pairing_camera)
                            val crop = preview.barcodeView.framingRect
                            ready = activity.resources.configuration.orientation == orientation &&
                                crop != null && crop.width() > 0 && crop.width() == crop.height() &&
                                crop.right <= preview.width && crop.bottom <= preview.height
                        }
                        if (!ready) SystemClock.sleep(100)
                    } while (!ready && SystemClock.elapsedRealtime() < deadline)
                    assertTrue("Square camera crop should fit orientation $orientation", ready)
                }
                scenario.onActivity { it.findViewById<android.view.View>(R.id.cancel_pairing_scan).performClick() }
            }
        } finally { automation.setRotation(UiAutomation.ROTATION_UNFREEZE) }
    }
}
