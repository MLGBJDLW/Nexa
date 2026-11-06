package com.nexa.mobile

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.journeyapps.barcodescanner.BarcodeView
import com.journeyapps.barcodescanner.CaptureActivity
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.journeyapps.barcodescanner.ViewfinderView
import kotlin.math.roundToInt

class PairingScannerActivity : CaptureActivity() {
    override fun initializeContent(): DecoratedBarcodeView {
        setContentView(R.layout.pairing_scanner)
        val root = findViewById<View>(R.id.pairing_root)
        ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
        findViewById<View>(R.id.cancel_pairing_scan).setOnClickListener { finish() }
        return findViewById(R.id.pairing_camera)
    }
}

class PairingBarcodeView(context: Context, attrs: AttributeSet) : BarcodeView(context, attrs) {
    override fun calculateFramingRect(container: Rect, surface: Rect): Rect {
        val visible = Rect(container)
        visible.intersect(surface)
        val side = minOf(visible.width() * .76f, visible.height() * .76f, resources.displayMetrics.density * 320).roundToInt()
        val left = visible.centerX() - side / 2
        val top = visible.centerY() - side / 2
        return Rect(left, top, left + side, top + side)
    }
}

/** The guide follows ZXing's actual decode crop, including after a rotation or window resize. */
class PairingViewfinder(context: Context, attrs: AttributeSet) : ViewfinderView(context, attrs) {
    private val outline = Path()
    private val bounds = RectF()
    private val density = resources.displayMetrics.density

    override fun onDraw(canvas: Canvas) {
        val frame = cameraPreview?.framingRect ?: return
        bounds.set(frame)
        outline.rewind()
        outline.addRoundRect(bounds, 18 * density, 18 * density, Path.Direction.CW)
        canvas.save()
        canvas.clipOutPath(outline)
        canvas.drawColor(0x990e1215.toInt())
        canvas.restore()

        paint.style = Paint.Style.STROKE
        paint.strokeWidth = density
        paint.color = 0x88ffffff.toInt()
        canvas.drawPath(outline, paint)
        val corner = 32 * density
        canvas.save()
        canvas.clipOutRect(bounds.left + corner, 0f, bounds.right - corner, height.toFloat())
        canvas.clipOutRect(0f, bounds.top + corner, width.toFloat(), bounds.bottom - corner)
        paint.strokeWidth = 3 * density
        paint.color = Color.WHITE
        canvas.drawPath(outline, paint)
        canvas.restore()
    }
}
