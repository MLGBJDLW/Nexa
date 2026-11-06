package com.nexa.mobile

import org.junit.Assert.*
import org.junit.Test

class OverlayDockMotionTest {
    @Test fun centerReleaseReturnsFasterWithStrongerRebound() {
        val center = OverlayDockMotion(179, 8, 352, 1f)
        val edge = OverlayDockMotion(28, 8, 352, 1f)
        assertTrue(center.durationMs < edge.durationMs)
        assertTrue(center.amplitude > edge.amplitude)
        assertTrue(center.x(.54f) - center.destination > edge.x(.54f) - edge.destination)
    }
    @Test fun reboundStaysReachableAndSettlesOnEitherEdge() {
        for (origin in 8..352) {
            val motion = OverlayDockMotion(origin, 8, 352, 1f)
            assertEquals(origin, motion.x(0f))
            for (frame in 0..100) assertTrue(motion.x(frame / 100f) in 8..352)
            assertEquals(motion.destination, motion.x(1f))
        }
    }
}
