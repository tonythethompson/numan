use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    pub keys: HashMap<String, TrustedKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedKey {
    pub fingerprint: String,
    pub public_key_b64: String,
    pub added_at: String,
}

impl TrustStore {
    pub fn load(root: &Path) -> Result<Self> {
        let trust_path = root.join("registry/trust.json");
        if !trust_path.exists() {
            return Ok(Self {
                keys: HashMap::new(),
            });
        }
        let content = std::fs::read_to_string(&trust_path)?;
        let store: TrustStore = serde_json::from_str(&content)?;
        Ok(store)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let trust_path = root.join("registry/trust.json");
        if let Some(parent) = trust_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&trust_path, content)?;
        Ok(())
    }

    pub fn add_key(&mut self, registry_name: &str, public_key_b64: &str) -> Result<String> {
        let key_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, public_key_b64)
                .context("Invalid base64 public key")?;

        if key_bytes.len() != 32 {
            bail!(
                "Ed25519 public key must be 32 bytes, got {}",
                key_bytes.len()
            );
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);

        let fingerprint = compute_fingerprint(&key_array);

        self.keys.insert(
            registry_name.to_string(),
            TrustedKey {
                fingerprint: fingerprint.clone(),
                public_key_b64: public_key_b64.to_string(),
                added_at: chrono_now(),
            },
        );

        Ok(fingerprint)
    }

    pub fn verify_signature(
        &self,
        registry_name: &str,
        data: &[u8],
        signature_b64: &str,
    ) -> Result<bool> {
        let key = self
            .keys
            .get(registry_name)
            .context(format!("No trusted key for registry '{registry_name}'"))?;

        let key_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &key.public_key_b64,
        )?;

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);

        let verifying_key = VerifyingKey::from_bytes(&key_array)?;

        let sig_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature_b64)?;

        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(&sig_bytes);

        let signature = Signature::from_bytes(&sig_array);

        Ok(verifying_key.verify(data, &signature).is_ok())
    }
}

pub fn compute_fingerprint(public_key: &[u8; 32]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

fn chrono_now() -> String {
    // Simple timestamp without pulling in chrono
    format!(
        "{:?}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    fn test_keypair() -> (SigningKey, VerifyingKey) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    #[test]
    fn compute_fingerprint_format() {
        let key = [1u8; 32];
        let fp = compute_fingerprint(&key);
        assert!(fp.starts_with("sha256:"));
        assert_eq!(fp.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn add_and_verify_key() {
        let (signing_key, verifying_key) = test_keypair();
        let public_key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &verifying_key.to_bytes(),
        );

        let mut store = TrustStore {
            keys: HashMap::new(),
        };

        let fingerprint = store.add_key("test", &public_key_b64).unwrap();
        assert!(fingerprint.starts_with("sha256:"));

        let data = b"test data to sign";
        let signature = signing_key.sign(data);
        let sig_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &signature.to_bytes(),
        );

        assert!(store.verify_signature("test", data, &sig_b64).unwrap());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let (_, verifying_key1) = test_keypair();
        let (signing_key2, _) = test_keypair();

        let public_key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &verifying_key1.to_bytes(),
        );

        let mut store = TrustStore {
            keys: HashMap::new(),
        };
        store.add_key("test", &public_key_b64).unwrap();

        let data = b"test data";
        let signature = signing_key2.sign(data);
        let sig_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &signature.to_bytes(),
        );

        assert!(!store.verify_signature("test", data, &sig_b64).unwrap());
    }

    #[test]
    fn verify_rejects_tampered_data() {
        let (signing_key, verifying_key) = test_keypair();
        let public_key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &verifying_key.to_bytes(),
        );

        let mut store = TrustStore {
            keys: HashMap::new(),
        };
        store.add_key("test", &public_key_b64).unwrap();

        let data = b"original data";
        let signature = signing_key.sign(data);
        let sig_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &signature.to_bytes(),
        );

        assert!(!store
            .verify_signature("test", b"tampered data", &sig_b64)
            .unwrap());
    }
}
