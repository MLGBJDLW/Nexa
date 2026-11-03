package com.nexa.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** AndroidKeyStore key material is never exported or sent to the WebView. */
class SecretStore(context: Context) {
    private val preferences = context.getSharedPreferences("nexa_secrets", Context.MODE_PRIVATE)
    private val alias = "nexa.mobile.credentials.v1"
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Synchronized fun write(name: String, value: String?) {
        require(name == "local-provider" || name == "remote-hosts")
        if (value == null) { check(preferences.edit().remove(name).commit()); return }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()); updateAAD(name.toByteArray()) }
        val encrypted = cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        check(preferences.edit().putString(name, Base64.encodeToString(cipher.iv + encrypted, Base64.NO_WRAP)).commit())
    }
    @Synchronized fun read(name: String): String? {
        require(name == "local-provider" || name == "remote-hosts")
        val encoded = preferences.getString(name, null) ?: return null
        val bytes = Base64.decode(encoded, Base64.NO_WRAP)
        require(bytes.size >= 28)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12))); updateAAD(name.toByteArray())
        }
        return String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8)
    }
}
