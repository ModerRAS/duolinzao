package com.duolinzao.app.auth

import android.app.Activity
import android.util.Log
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetCredentialResponse
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.NoCredentialException
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.CreateCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreateCredentialResponse

/**
 * Passkey（Credential Manager 平台 API，禁止 WebView）。
 * 任何失败（无可用凭据、无凭据提供方/无 GMS、用户取消、断言失败）都返回 null，
 * 调用方自动回退密码，不卡死、不静默重试。
 */
object PasskeyHelper {
    private const val TAG = "Passkey"

    /** 登录断言：requestJson 为服务端 publicKey 原样 JSON。成功返回 credential JSON（webauthn_rs 形状）。 */
    suspend fun getAssertion(activity: Activity, requestJson: String): String? {
        return try {
            val manager = CredentialManager.create(activity)
            val option = GetPublicKeyCredentialOption(requestJson)
            val request = GetCredentialRequest(listOf(option))
            val response: GetCredentialResponse = manager.getCredential(activity, request)
            response.credential.data.getString("androidx.credentials.BUNDLE_KEY_AUTHENTICATION_RESPONSE_JSON")
        } catch (e: GetCredentialCancellationException) {
            Log.i(TAG, "user cancelled")
            null
        } catch (e: NoCredentialException) {
            Log.i(TAG, "no credential: ${e.message}")
            null
        } catch (e: Exception) {
            Log.i(TAG, "assertion failed: ${e.message}")
            null
        }
    }

    /** 绑定：requestJson 为服务端 register/start 的 publicKey 原样 JSON。成功返回 credential JSON。 */
    suspend fun createPasskey(activity: Activity, requestJson: String): String? {
        return try {
            val manager = CredentialManager.create(activity)
            val request: CreateCredentialRequest = CreatePublicKeyCredentialRequest(requestJson)
            val response: CreateCredentialResponse = manager.createCredential(activity, request)
            response.data.getString("androidx.credentials.BUNDLE_KEY_REGISTRATION_RESPONSE_JSON")
        } catch (e: Exception) {
            Log.i(TAG, "create failed: ${e.message}")
            null
        }
    }
}
