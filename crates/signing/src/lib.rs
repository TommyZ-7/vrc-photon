//! Shared minisign verification for update artifacts and AI plugin manifests.
//!
//! Callers own key discovery and the mapping to their public error type. This
//! crate only parses the detached signature, streams the data through the
//! verifier, and reports a signature or IO failure.

use std::io::Read;

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use thiserror::Error;

/// A minisign verification failure.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// The signature, key list, or signature bytes were invalid.
    #[error("invalid minisign signature")]
    Signature,
    /// The caller's reader failed while streaming the signed data.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Verifies a detached minisign signature over an in-memory byte slice.
pub fn verify_bytes(data: &[u8], signature_text: &str, keys: &[String]) -> Result<(), VerifyError> {
    verify_reader(&mut std::io::Cursor::new(data), signature_text, keys)
}

/// Verifies a detached minisign signature over a streaming reader.
pub fn verify_reader<R: Read>(
    reader: &mut R,
    signature_text: &str,
    keys: &[String],
) -> Result<(), VerifyError> {
    let signature = Signature::decode(signature_text).map_err(|_| VerifyError::Signature)?;
    let public_keys: Vec<PublicKey> = keys
        .iter()
        .filter_map(|key| PublicKey::from_base64(key.trim()).ok())
        .collect();
    if public_keys.is_empty() {
        return Err(VerifyError::Signature);
    }

    let mut verifier = public_keys
        .iter()
        .find_map(|key| key.verify_stream(&signature).ok())
        .ok_or(VerifyError::Signature)?;

    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }

    verifier.finalize().map_err(|_| VerifyError::Signature)
}

/// Decodes the base64 representation used in manifest `signature` fields.
pub fn decode_signature_base64(encoded: &str) -> Result<String, VerifyError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| VerifyError::Signature)?;
    String::from_utf8(bytes).map_err(|_| VerifyError::Signature)
}

/// Parses a key file. Blank lines and comments are ignored; one key per line.
pub fn parse_public_keys(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with('#') && !line.starts_with("untrusted comment:")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

    #[test]
    fn rejects_a_malformed_signature_even_with_a_valid_key() {
        assert!(matches!(
            verify_bytes(b"data", "not a signature", &[PUBLIC_KEY.to_string()]),
            Err(VerifyError::Signature)
        ));
    }

    #[test]
    fn parses_key_files_and_ignores_comments() {
        let text = format!("# comment\n\nuntrusted comment: key\n{PUBLIC_KEY}\n");
        assert_eq!(parse_public_keys(&text), vec![PUBLIC_KEY.to_string()]);
        assert!(parse_public_keys("  \n# only comments\n").is_empty());
    }

    #[test]
    fn decodes_base64_signature_text_and_rejects_invalid_input() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"signature");
        assert_eq!(decode_signature_base64(&encoded).unwrap(), "signature");
        assert!(matches!(
            decode_signature_base64("not base64!"),
            Err(VerifyError::Signature)
        ));
    }
}
