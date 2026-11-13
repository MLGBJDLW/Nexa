package com.nexa.mobile

import org.junit.Assert.*
import org.junit.Test

class PhoneScreenEvidenceTest {
    private val editor = Any()
    private val original = PhoneScreenEvidence("app.test", 7, 0, 12L, listOf(0, 20, 1080, 2200), listOf(listOf(0, 1600, 1080, 2200)),
        540, 1100, 1080, 2200, "masked-pixel-fingerprint", listOf("node0:text:focused:editable"), "field1:text")
    private fun refreshed(current: PhoneScreenEvidence, previous: PhoneScreenEvidence? = original, currentEditor: Any? = editor, sameControls: Boolean = true) =
        retainPhoneObservationId("clicked-frame", previous, editor, "new-frame", current, currentEditor, sameControls)

    @Test fun heldClickRemainsValidAcrossUnchangedPreviewCapture() {
        assertEquals("clicked-frame", refreshed(original.copy(controls = original.controls.toList())))
    }
    @Test fun changingAnyCapturedTargetEvidenceRequiresNewObservation() {
        val changes = listOf(original.copy(packageName="other.app"), original.copy(windowId=8),
            original.copy(rotation=1), original.copy(revision=13L), original.copy(appBounds=listOf(0,0,1080,2200)),
            original.copy(maskedBounds=emptyList()), original.copy(imageWidth=1080), original.copy(imageHeight=2200), original.copy(displayWidth=1440),
            original.copy(displayHeight=2400), original.copy(fingerprint="different-pixels"),
            original.copy(controls=listOf("node0:other-editor:focused:editable")), original.copy(editorSignature="field2:text"))
        changes.forEach { assertEquals("new-frame", refreshed(it)) }
    }
    @Test fun identicalLookingReplacementEditorCannotInheritInput() {
        assertEquals("new-frame", refreshed(original, currentEditor=Any()))
        assertEquals("new-frame", refreshed(original, sameControls=false))
    }
    @Test fun explicitObservationActionOrCancellationCannotReviveOldEvidence() {
        assertEquals("new-frame", refreshed(original, previous=null))
        assertEquals("new-frame", retainPhoneObservationId("", original, editor, "new-frame", original, editor, true))
    }
}
