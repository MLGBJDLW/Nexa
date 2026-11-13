package com.nexa.mobile

/** In-memory identity only; never persist native nodes, editor objects or pixels. */
internal data class PhoneScreenEvidence(
    val packageName: String,
    val windowId: Int,
    val rotation: Int,
    val revision: Long,
    val appBounds: List<Int>,
    val maskedBounds: List<List<Int>>,
    val imageWidth: Int,
    val imageHeight: Int,
    val displayWidth: Int,
    val displayHeight: Int,
    val fingerprint: String,
    val controls: List<String>,
    val editorSignature: String?,
)

/** A preview refresh cannot revoke a queued click if all native evidence is unchanged. */
internal fun retainPhoneObservationId(
    previousId: String,
    previous: PhoneScreenEvidence?,
    previousEditor: Any?,
    nextId: String,
    current: PhoneScreenEvidence,
    currentEditor: Any?,
    nativeControlsUnchanged: Boolean,
): String = if (previousId.isNotEmpty() && previous == current && previousEditor === currentEditor && nativeControlsUnchanged) previousId else nextId
