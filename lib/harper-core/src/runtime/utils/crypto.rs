// Copyright 2026 coccinella-labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(dead_code)]

use crate::core::constants::crypto;
use crate::core::error::{HarperError, HarperResult};
use aes::{
    Aes256,
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7},
};
use cbc::{Decryptor, Encryptor};
use ctr::{Ctr64BE, cipher::StreamCipher};
use ring::{
    aead, digest,
    rand::{SecureRandom, SystemRandom},
};

/// Type alias for the return type of encrypt_message
pub type EncryptionResult = HarperResult<(Vec<u8>, Vec<u8>, Vec<u8>)>;

pub struct CryptoUtils;

impl CryptoUtils {
    /// Generate a random secret key
    pub fn generate_secret_key() -> HarperResult<Vec<u8>> {
        let rng = SystemRandom::new();
        let mut key = vec![0u8; crypto::AES_256_KEY_LEN];
        rng.fill(&mut key)
            .map_err(|e| HarperError::Crypto(format!("Key generation failed: {}", e)))?;
        Ok(key)
    }

    /// Generate a cryptographic hash of data
    pub fn hash_data(data: &[u8]) -> Vec<u8> {
        let digest = digest::digest(&digest::SHA256, data);
        digest.as_ref().to_vec()
    }

    /// Generate a zero-knowledge proof (simplified demonstration)
    pub fn generate_zk_proof(
        secret: &[u8],
        public_data: &[u8],
    ) -> HarperResult<(Vec<u8>, Vec<u8>)> {
        // Simplified ZK proof demonstration
        let _commitment = Self::hash_data(secret);
        let challenge = Self::hash_data(public_data);

        // Response = hash(secret + challenge)
        let mut response_input = secret.to_vec();
        response_input.extend_from_slice(&challenge);
        let response = Self::hash_data(&response_input);

        Ok((challenge, response))
    }

    /// Verify a zero-knowledge proof
    pub fn verify_zk_proof(
        public_data: &[u8],
        challenge: &[u8],
        _response: &[u8],
    ) -> HarperResult<bool> {
        // Verify that response = hash(??? + challenge)
        // This is a simplified demonstration
        let expected_challenge = Self::hash_data(public_data);
        Ok(challenge == expected_challenge.as_slice())
    }

    /// Helper function to generate a nonce
    fn generate_nonce() -> HarperResult<aead::Nonce> {
        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; crypto::AES_GCM_NONCE_LEN];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| HarperError::Crypto("Nonce generation failed".to_string()))?;
        Ok(aead::Nonce::assume_unique_for_key(nonce_bytes))
    }

    /// Encrypt data using AES-CBC block cipher
    pub fn encrypt_block_cipher(data: &[u8]) -> HarperResult<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        if data.is_empty() {
            return Err(HarperError::Crypto("empty data".to_string()));
        }

        let rng = SystemRandom::new();

        let mut key = vec![0u8; 32];
        rng.fill(&mut key)
            .map_err(|e| HarperError::Crypto(format!("Key generation failed: {}", e)))?;

        let mut iv = vec![0u8; 16];
        rng.fill(&mut iv)
            .map_err(|e| HarperError::Crypto(format!("IV generation failed: {}", e)))?;

        let cipher = Encryptor::<Aes256>::new_from_slices(&key, &iv)
            .map_err(|_| HarperError::Crypto("Invalid key or IV".to_string()))?;

        let block_size = 16;
        let padded_len = data.len().div_ceil(block_size) * block_size;
        let mut buf = vec![0u8; padded_len];
        buf[..data.len()].copy_from_slice(data);
        let encrypted = cipher
            .encrypt_padded_mut::<Pkcs7>(&mut buf, data.len())
            .map_err(|_| HarperError::Crypto("Encryption failed".to_string()))?;

        Ok((encrypted.to_vec(), key, iv))
    }

    /// Decrypt data using AES-CBC block cipher
    pub fn decrypt_block_cipher(encrypted: &[u8], key: &[u8], iv: &[u8]) -> HarperResult<Vec<u8>> {
        if encrypted.is_empty() {
            return Err(HarperError::Crypto("empty data".to_string()));
        }

        if key.len() != 32 {
            return Err(HarperError::Crypto("invalid key length".to_string()));
        }

        if iv.len() != 16 {
            return Err(HarperError::Crypto("invalid IV length".to_string()));
        }

        let cipher = Decryptor::<Aes256>::new_from_slices(key, iv)
            .map_err(|_| HarperError::Crypto("Invalid key or IV".to_string()))?;

        let mut buf = encrypted.to_vec();
        let decrypted = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut buf)
            .map_err(|_| HarperError::Crypto("decryption failed".to_string()))?;

        Ok(decrypted.to_vec())
    }

    /// Encrypt data using AES-CTR block cipher mode
    pub fn encrypt_block_cipher_ctr(data: &[u8]) -> HarperResult<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        if data.is_empty() {
            return Err(HarperError::Crypto("empty data".to_string()));
        }

        let rng = SystemRandom::new();

        let mut key = vec![0u8; 32];
        rng.fill(&mut key)
            .map_err(|e| HarperError::Crypto(format!("Key generation failed: {}", e)))?;

        let mut nonce = vec![0u8; 16];
        rng.fill(&mut nonce)
            .map_err(|e| HarperError::Crypto(format!("Nonce generation failed: {}", e)))?;

        let mut cipher = Ctr64BE::<Aes256>::new(key[..].into(), nonce[..].into());

        let mut result = data.to_vec();
        cipher.apply_keystream(&mut result);

        Ok((result, key, nonce))
    }

    /// Decrypt data using AES-CTR (same as encrypt)
    pub fn decrypt_block_cipher_ctr(
        encrypted: &[u8],
        key: &[u8],
        nonce: &[u8],
    ) -> HarperResult<Vec<u8>> {
        if encrypted.is_empty() {
            return Err(HarperError::Crypto("empty data".to_string()));
        }

        if key.len() != 32 {
            return Err(HarperError::Crypto("invalid key length".to_string()));
        }

        if nonce.len() != 16 {
            return Err(HarperError::Crypto("invalid nonce length".to_string()));
        }

        let mut cipher = Ctr64BE::<Aes256>::new(key[..].into(), nonce[..].into());

        let mut result = encrypted.to_vec();
        cipher.apply_keystream(&mut result);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::constants::crypto;

    #[test]
    fn test_crypto_operations() -> HarperResult<()> {
        // Test key generation
        let secret = CryptoUtils::generate_secret_key()?;
        assert_eq!(secret.len(), crypto::AES_256_KEY_LEN);

        // Test hashing
        let message = b"Hello, world!";
        let hash = CryptoUtils::hash_data(message);
        assert_eq!(hash.len(), crypto::SHA256_LEN);

        // Test ZK proof generation and verification
        let secret_data = b"secret";
        let public_data = b"public";
        let (challenge, response) = CryptoUtils::generate_zk_proof(secret_data, public_data)?;
        let is_valid = CryptoUtils::verify_zk_proof(public_data, &challenge, &response)?;
        assert!(is_valid);

        Ok(())
    }

    #[test]
    fn test_block_cipher_encrypt_decrypt() -> HarperResult<()> {
        let message = b"Hello, block cipher world!";
        let (encrypted, key, iv) = CryptoUtils::encrypt_block_cipher(message)?;

        assert!(!encrypted.is_empty());
        assert_eq!(key.len(), 32);
        assert_eq!(iv.len(), 16);

        let decrypted = CryptoUtils::decrypt_block_cipher(&encrypted, &key, &iv)?;

        assert_eq!(decrypted, message);

        Ok(())
    }

    #[test]
    fn test_block_cipher_ctr_encrypt_decrypt() -> HarperResult<()> {
        let message = b"Hello, CTR block cipher world!";
        let (encrypted, key, nonce) = CryptoUtils::encrypt_block_cipher_ctr(message)?;

        assert!(!encrypted.is_empty());
        assert_eq!(encrypted.len(), message.len()); // No padding
        assert_eq!(key.len(), 32);
        assert_eq!(nonce.len(), 16);

        let decrypted = CryptoUtils::decrypt_block_cipher_ctr(&encrypted, &key, &nonce)?;

        assert_eq!(decrypted, message);

        Ok(())
    }
}
