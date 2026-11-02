use std::sync::{Arc, OnceLock};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::error::AppError;

/// Argon2id 约 19 MiB。计算放在 blocking 线程，避免堵住 async worker。
/// tokio 的 blocking 池默认可开很多线程，所以同时最多两个，避免把 256 MiB 容器打满。
const PASSWORD_KDF_PERMITS: usize = 2;

pub fn sha256_hex(data: &str) -> String {
    let digest = Sha256::digest(data.as_bytes());
    hex::encode(digest)
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

fn argon() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn password_kdf_permits() -> &'static Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS.get_or_init(|| Arc::new(Semaphore::new(PASSWORD_KDF_PERMITS)))
}

/// 许可跟计算走完才释放。请求取消后 blocking 任务仍会跑完，不能提前把名额让出去。
async fn with_password_kdf<T, F>(work: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = password_kdf_permits()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::internal())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| AppError::internal())
}

fn hash_password_blocking(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let hash = argon()
        .hash_password(password.as_bytes(), &salt)?
        .to_string();
    Ok(hash)
}

fn verify_password_blocking(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    argon()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn hash_password(password: &str) -> Result<String, AppError> {
    let password = password.to_owned();
    with_password_kdf(move || hash_password_blocking(&password)).await?
}

pub async fn verify_password(password: &str, encoded: &str) -> Result<bool, AppError> {
    let password = password.to_owned();
    let encoded = encoded.to_owned();
    with_password_kdf(move || verify_password_blocking(&password, &encoded)).await
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|_| AppError::internal())?;
    let mut out = nonce_bytes.to_vec();
    out.extend(ciphertext);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], packed: &[u8]) -> Result<Vec<u8>, AppError> {
    if packed.len() < 13 {
        return Err(AppError::internal());
    }
    let (nonce, ciphertext) = packed.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| AppError::internal())
}

pub fn decode_local_key(raw: &str) -> Option<[u8; 32]> {
    let trimmed = raw.trim();
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Some(key);
        }
    }
    if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(trimmed) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Some(key);
        }
    }
    if let Ok(bytes) = hex::decode(trimmed) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Some(key);
        }
    }
    None
}

pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(("image/png", "png"));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[tokio::test]
    async fn password_hash_roundtrip() {
        let encoded = hash_password("correct-horse").await.expect("hash");
        assert!(verify_password("correct-horse", &encoded)
            .await
            .expect("verify"));
        assert!(!verify_password("wrong-horse", &encoded)
            .await
            .expect("verify"));
        assert!(!verify_password("correct-horse", "not-a-hash")
            .await
            .expect("verify"));
    }

    /// 当前线程运行时上，哈希若直接算在 worker 里，5ms 的 sleep 排不到。
    #[tokio::test(flavor = "current_thread")]
    async fn password_hash_lets_the_runtime_continue() {
        let hasher = hash_password("correct-horse");
        tokio::pin!(hasher);
        let mut ticked = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > deadline {
                panic!("password hash hung");
            }
            tokio::select! {
                biased;
                encoded = &mut hasher => {
                    let encoded = encoded.expect("hash");
                    assert!(encoded.starts_with("$argon2id$"));
                    assert!(ticked, "runtime did not proceed during password hash");
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(5)), if !ticked => {
                    ticked = true;
                }
            }
        }
    }
}
