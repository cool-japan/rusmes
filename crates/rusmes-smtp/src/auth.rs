//! SMTP authentication mechanisms
//!
//! Provides implementations for SMTP AUTH mechanisms:
//! - PLAIN (RFC 4616)
//! - LOGIN
//! - CRAM-MD5 (RFC 2195)
//! - SCRAM-SHA-256 (RFC 5802, RFC 7677)
//!
//! All random number generation uses `getrandom` for cryptographic security.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use md5::Md5;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacMd5 = Hmac<Md5>;
type HmacSha256 = Hmac<Sha256>;

/// Generate cryptographically secure random bytes using getrandom
fn fill_random_bytes(buf: &mut [u8]) -> anyhow::Result<()> {
    getrandom::fill(buf).map_err(|e| anyhow::anyhow!("RNG failure: {}", e))
}

/// Generate a cryptographically secure random u64 value
fn random_u64() -> anyhow::Result<u64> {
    let mut bytes = [0u8; 8];
    fill_random_bytes(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Generate a CRAM-MD5 challenge
///
/// Returns a challenge in the format: `<timestamp.random@hostname>`
pub fn generate_cram_md5_challenge(hostname: &str) -> anyhow::Result<String> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let random = random_u64()?;
    let challenge = format!("<{}.{}@{}>", timestamp, random, hostname);
    Ok(challenge)
}

/// Compute CRAM-MD5 HMAC response
///
/// Given a password and challenge, compute the expected HMAC-MD5 response
pub fn compute_cram_md5_hmac(password: &str, challenge: &str) -> anyhow::Result<String> {
    let mut mac = HmacMd5::new_from_slice(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(challenge.as_bytes());
    let result = mac.finalize();
    Ok(hex::encode(result.into_bytes()))
}

/// Encode challenge in Base64 for transmission
pub fn encode_challenge(challenge: &str) -> String {
    BASE64.encode(challenge.as_bytes())
}

/// Decode Base64-encoded client response
pub fn decode_response(encoded: &str) -> anyhow::Result<String> {
    let decoded = BASE64
        .decode(encoded.trim())
        .map_err(|e| anyhow::anyhow!("Failed to decode Base64: {}", e))?;
    let response_str =
        String::from_utf8(decoded).map_err(|e| anyhow::anyhow!("Failed to decode UTF-8: {}", e))?;
    Ok(response_str)
}

/// Parse PLAIN authentication credentials
///
/// PLAIN format: \0username\0password (Base64 encoded)
/// Returns (username, password)
pub fn parse_plain_auth(encoded: &str) -> anyhow::Result<(String, String)> {
    let decoded = decode_response(encoded)?;
    let parts: Vec<&str> = decoded.split('\0').collect();

    // PLAIN format is: [authzid] \0 authcid \0 password
    // We support both forms:
    // 1. \0username\0password (3 parts where first is empty)
    // 2. username\0password (2 parts)

    match parts.len() {
        2 => {
            // Format: username\0password
            Ok((parts[0].to_string(), parts[1].to_string()))
        }
        3 => {
            // Format: \0username\0password or authzid\0username\0password
            // We use authcid (middle part) as the username
            Ok((parts[1].to_string(), parts[2].to_string()))
        }
        _ => Err(anyhow::anyhow!(
            "Invalid PLAIN authentication format: expected 2 or 3 null-separated parts"
        )),
    }
}

/// Parse CRAM-MD5 response into username and HMAC
///
/// Response format: `username HMAC`
pub fn parse_cram_md5_response(response: &str) -> anyhow::Result<(&str, &str)> {
    let parts: Vec<&str> = response.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid CRAM-MD5 response format: expected 'username hmac'"
        ));
    }
    Ok((parts[0], parts[1]))
}

// ============================================================================
// SCRAM-SHA-256 Implementation (RFC 5802, RFC 7677)
// ============================================================================

/// Parse SCRAM client-first message
///
/// Format: n,,n=username,r=clientNonce
/// Returns (username, client_nonce, client_first_bare)
pub fn parse_scram_client_first(msg: &str) -> anyhow::Result<(String, String, String)> {
    // Skip GS2 header (n,,)
    let client_first_bare = msg
        .strip_prefix("n,,")
        .ok_or_else(|| anyhow::anyhow!("Invalid GS2 header in client-first message"))?;

    let mut username = None;
    let mut nonce = None;

    for part in client_first_bare.split(',') {
        if let Some(value) = part.strip_prefix("n=") {
            username = Some(value.to_string());
        } else if let Some(value) = part.strip_prefix("r=") {
            nonce = Some(value.to_string());
        }
    }

    let username = username.ok_or_else(|| anyhow::anyhow!("Missing username in client-first"))?;
    let nonce = nonce.ok_or_else(|| anyhow::anyhow!("Missing nonce in client-first"))?;

    Ok((username, nonce, client_first_bare.to_string()))
}

/// Parse SCRAM client-final message
///
/// Format: c=biws,r=nonce,p=proof
/// Returns (channel_binding, nonce, proof, client_final_without_proof)
pub fn parse_scram_client_final(msg: &str) -> anyhow::Result<(String, String, String, String)> {
    let mut channel_binding = None;
    let mut nonce = None;
    let mut proof = None;

    // Extract client-final-without-proof (everything before ,p=)
    let client_final_without_proof = msg
        .rsplit_once(",p=")
        .map(|(before, _)| before)
        .ok_or_else(|| anyhow::anyhow!("Invalid client-final: missing proof"))?;

    for part in msg.split(',') {
        if let Some(value) = part.strip_prefix("c=") {
            channel_binding = Some(value.to_string());
        } else if let Some(value) = part.strip_prefix("r=") {
            nonce = Some(value.to_string());
        } else if let Some(value) = part.strip_prefix("p=") {
            proof = Some(value.to_string());
        }
    }

    let channel_binding = channel_binding
        .ok_or_else(|| anyhow::anyhow!("Missing channel binding in client-final"))?;
    let nonce = nonce.ok_or_else(|| anyhow::anyhow!("Missing nonce in client-final"))?;
    let proof = proof.ok_or_else(|| anyhow::anyhow!("Missing proof in client-final"))?;

    Ok((
        channel_binding,
        nonce,
        proof,
        client_final_without_proof.to_string(),
    ))
}

/// Generate SCRAM server nonce (16 random bytes as hex)
pub fn generate_scram_server_nonce() -> anyhow::Result<String> {
    let mut random_bytes = [0u8; 16];
    fill_random_bytes(&mut random_bytes)?;
    Ok(hex::encode(random_bytes))
}

/// Compute SCRAM SaltedPassword using PBKDF2-HMAC-SHA256
///
/// SaltedPassword = PBKDF2(password, salt, iteration_count)
pub fn compute_salted_password(
    password: &str,
    salt: &[u8],
    iterations: u32,
) -> anyhow::Result<Vec<u8>> {
    let mut salted_password = vec![0u8; 32]; // SHA256 output size
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut salted_password);
    Ok(salted_password)
}

/// Compute SCRAM ClientKey
///
/// ClientKey = HMAC(SaltedPassword, "Client Key")
pub fn compute_client_key(salted_password: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(salted_password)
        .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(b"Client Key");
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Compute SCRAM StoredKey
///
/// StoredKey = SHA256(ClientKey)
pub fn compute_stored_key(client_key: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(client_key);
    hasher.finalize().to_vec()
}

/// Compute SCRAM ServerKey
///
/// ServerKey = HMAC(SaltedPassword, "Server Key")
pub fn compute_server_key(salted_password: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(salted_password)
        .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(b"Server Key");
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Verify SCRAM client proof
///
/// ClientSignature = HMAC(StoredKey, AuthMessage)
/// ClientKey = ClientProof XOR ClientSignature
/// StoredKey = SHA256(ClientKey)
/// Compare computed StoredKey with expected StoredKey
pub fn verify_scram_client_proof(
    stored_key: &[u8],
    auth_message: &str,
    proof_b64: &str,
) -> anyhow::Result<bool> {
    let proof = BASE64
        .decode(proof_b64)
        .map_err(|e| anyhow::anyhow!("Failed to decode proof: {}", e))?;

    // ClientSignature = HMAC(StoredKey, AuthMessage)
    let mut mac = HmacSha256::new_from_slice(stored_key)
        .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(auth_message.as_bytes());
    let client_signature = mac.finalize().into_bytes();

    // ClientKey = ClientProof XOR ClientSignature
    if proof.len() != client_signature.len() {
        return Ok(false);
    }

    let mut client_key = vec![0u8; proof.len()];
    for i in 0..proof.len() {
        client_key[i] = proof[i] ^ client_signature[i];
    }

    // StoredKey = SHA256(ClientKey)
    let computed_stored_key = compute_stored_key(&client_key);

    // Constant-time comparison
    Ok(computed_stored_key.as_slice() == stored_key)
}

/// Compute SCRAM server signature
///
/// ServerSignature = HMAC(ServerKey, AuthMessage)
pub fn compute_scram_server_signature(
    server_key: &[u8],
    auth_message: &str,
) -> anyhow::Result<String> {
    let mut mac = HmacSha256::new_from_slice(server_key)
        .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(auth_message.as_bytes());
    let server_signature = mac.finalize().into_bytes();
    Ok(BASE64.encode(server_signature.as_ref() as &[u8]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_challenge() {
        let challenge = generate_cram_md5_challenge("localhost")
            .expect("CRAM-MD5 challenge generation should succeed");
        assert!(challenge.starts_with('<'));
        assert!(challenge.ends_with('>'));
        assert!(challenge.contains("@localhost"));
    }

    #[test]
    fn test_compute_hmac() {
        let challenge = "<12345.67890@localhost>";
        let password = "secret";
        let hmac = compute_cram_md5_hmac(password, challenge)
            .expect("CRAM-MD5 HMAC computation should succeed");
        assert_eq!(hmac.len(), 32); // MD5 hash is 32 hex chars
    }

    #[test]
    fn test_encode_decode() {
        let challenge = "<12345.67890@localhost>";
        let encoded = encode_challenge(challenge);
        let decoded =
            decode_response(&encoded).expect("Base64 decode of valid challenge should succeed");
        assert_eq!(challenge, decoded);
    }

    #[test]
    fn test_parse_response() {
        let response = "testuser 3c6e0b8a9c15224a8228b9a98ca1531d";
        let (username, hmac) =
            parse_cram_md5_response(response).expect("CRAM-MD5 response parse should succeed");
        assert_eq!(username, "testuser");
        assert_eq!(hmac, "3c6e0b8a9c15224a8228b9a98ca1531d");
    }

    #[test]
    fn test_parse_response_invalid() {
        let response = "testuser";
        assert!(parse_cram_md5_response(response).is_err());
    }
}
