package dev.zerohop.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.ProviderException
import java.security.SecureRandom
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Keeps the secret that seals the device key on disk. The secret is stored
 * encrypted by a key that never leaves the Android Keystore, so a copy of this
 * app's files is useless on its own. No screen lock is required to use it,
 * because the app has to be able to receive with the phone in a pocket.
 */
object KeyVault {
    private const val ALIAS = "zerohop_device_seal"
    private const val FILE = "device_seal.bin"
    private const val PROVIDER = "AndroidKeyStore"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val SECRET_SIZE = 32
    private const val IV_SIZE = 12
    private const val TAG_BITS = 128

    /**
     * Null means a problem that may pass, and nothing is replaced then: a new
     * secret would make the existing device key impossible to open.
     */
    fun secret(context: Context): ByteArray? {
        val file = File(context.noBackupFilesDir, FILE)
        return try {
            val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
            val key = store.getKey(ALIAS, null) as? SecretKey
            if (key == null || !file.exists()) {
                create(store, file)
            } else {
                try {
                    open(key, file.readBytes()) ?: create(store, file)
                } catch (_: AEADBadTagException) {
                    create(store, file)
                } catch (_: KeyPermanentlyInvalidatedException) {
                    create(store, file)
                }
            }
        } catch (_: GeneralSecurityException) {
            null
        } catch (_: ProviderException) {
            null
        } catch (_: IOException) {
            null
        }
    }

    private fun open(key: SecretKey, stored: ByteArray): ByteArray? {
        if (stored.size != IV_SIZE + SECRET_SIZE + TAG_BITS / 8) return null
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, stored, 0, IV_SIZE))
        return cipher.doFinal(stored, IV_SIZE, stored.size - IV_SIZE)
    }

    private fun create(store: KeyStore, file: File): ByteArray {
        store.deleteEntry(ALIAS)
        val key = generateKey()
        val secret = ByteArray(SECRET_SIZE).also { SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key)
        val sealed = cipher.iv + cipher.doFinal(secret)
        val temporary = File(file.parentFile, "$FILE.tmp")
        temporary.outputStream().use {
            it.write(sealed)
            it.fd.sync()
        }
        if (!temporary.renameTo(file)) throw IOException("could not store the sealed secret")
        return secret
    }

    private fun generateKey(): SecretKey =
        try {
            generate(strongBox = true)
        } catch (_: ProviderException) {
            // Most phones have no separate security chip.
            generate(strongBox = false)
        }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(SECRET_SIZE * 8)
        if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            spec.setIsStrongBoxBacked(true)
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        generator.init(spec.build())
        return generator.generateKey()
    }
}
