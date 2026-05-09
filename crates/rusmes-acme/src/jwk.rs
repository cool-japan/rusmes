//! JWK (JSON Web Key) thumbprint computation per RFC 7638
//!
//! RFC 7638 defines the JWK Thumbprint as the SHA-256 digest of the canonical
//! minimal JSON representation of the key, encoded as URL-safe base64 with no
//! padding.  The canonical form includes only the required members for each key
//! type, listed in **lexicographic order**:
//!
//! - RSA: `e`, `kty`, `n`
//! - EC:  `crv`, `kty`, `x`, `y`
//!
//! No whitespace is permitted between tokens.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

/// A JSON Web Key, carrying only the fields required for thumbprint computation.
///
/// Binary fields (`e`, `n`, `x`, `y`) must already be base64url-encoded (no
/// padding) as they appear in standard JWK serialisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Jwk {
    /// RSA public key.  `e` = public exponent, `n` = modulus.
    Rsa {
        /// Base64url-encoded public exponent.
        e: String,
        /// Base64url-encoded RSA modulus.
        n: String,
    },
    /// Elliptic-curve public key.
    Ec {
        /// Curve name, e.g. `"P-256"`.
        crv: String,
        /// Base64url-encoded x-coordinate.
        x: String,
        /// Base64url-encoded y-coordinate.
        y: String,
    },
}

/// Build the RFC 7638 canonical JSON for an RSA key.
///
/// Members are in lexicographic order: `e`, `kty`, `n`.  No whitespace.
/// Base64url values contain only `[A-Za-z0-9_-]`, so no JSON escaping is
/// needed.
fn canonical_rsa_json(e: &str, n: &str) -> String {
    format!(r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#, e, n)
}

/// Build the RFC 7638 canonical JSON for an EC key.
///
/// Members are in lexicographic order: `crv`, `kty`, `x`, `y`.  No whitespace.
fn canonical_ec_json(crv: &str, x: &str, y: &str) -> String {
    format!(r#"{{"crv":"{}","kty":"EC","x":"{}","y":"{}"}}"#, crv, x, y)
}

/// Compute the RFC 7638 JWK Thumbprint for the given key.
///
/// Returns a 43-character URL-safe base64 string (no padding) that uniquely
/// identifies the key.
///
/// # Algorithm
///
/// 1. Serialise the key as canonical JSON (required members, lexicographic
///    order, no whitespace).
/// 2. Hash the UTF-8 bytes with SHA-256.
/// 3. Encode the 32-byte digest as URL-safe base64 with no padding.
pub fn jwk_thumbprint(jwk: &Jwk) -> String {
    let canonical = match jwk {
        Jwk::Rsa { e, n } => canonical_rsa_json(e, n),
        Jwk::Ec { crv, x, y } => canonical_ec_json(crv, x, y),
    };

    let hash = Sha256::digest(canonical.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.as_ref() as &[u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7638 §3.1 appendix A test vector.
    ///
    /// The expected thumbprint is `NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs`.
    #[test]
    fn jwk_thumbprint_rsa_rfc7638_appendix_a1() {
        let e = "AQAB";
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";

        let jwk = Jwk::Rsa {
            e: e.to_string(),
            n: n.to_string(),
        };

        let thumbprint = jwk_thumbprint(&jwk);
        assert_eq!(
            thumbprint, "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs",
            "RFC 7638 §3.1 test vector mismatch — check field order in canonical JSON"
        );
    }

    /// Verify that the thumbprint of two `Jwk::Rsa` values constructed with
    /// identical field contents is always the same, regardless of which field
    /// was provided first (the enum variant fixes the order).
    #[test]
    fn jwk_thumbprint_canonical_ordering() {
        let e = "AQAB";
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";

        // Two independent instances with identical fields.
        let jwk_a = Jwk::Rsa {
            e: e.to_string(),
            n: n.to_string(),
        };
        let jwk_b = Jwk::Rsa {
            e: e.to_string(),
            n: n.to_string(),
        };

        assert_eq!(
            jwk_thumbprint(&jwk_a),
            jwk_thumbprint(&jwk_b),
            "Identical JWKs must produce identical thumbprints"
        );

        // Also confirm the canonical JSON has the required lexicographic order:
        // e appears before kty, kty appears before n.
        let canonical = canonical_rsa_json(e, n);
        let e_pos = canonical.find("\"e\"").expect("e member missing");
        let kty_pos = canonical.find("\"kty\"").expect("kty member missing");
        let n_pos = canonical.find("\"n\"").expect("n member missing");
        assert!(
            e_pos < kty_pos && kty_pos < n_pos,
            "Canonical RSA JSON must order members e < kty < n; got positions {e_pos}, {kty_pos}, {n_pos}"
        );
    }

    /// The thumbprint must never contain a `=` padding character.
    #[test]
    fn jwk_thumbprint_no_padding() {
        let jwk_rsa = Jwk::Rsa {
            e: "AQAB".to_string(),
            n: "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string(),
        };

        let jwk_ec = Jwk::Ec {
            crv: "P-256".to_string(),
            x: "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU".to_string(),
            y: "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0".to_string(),
        };

        let tp_rsa = jwk_thumbprint(&jwk_rsa);
        let tp_ec = jwk_thumbprint(&jwk_ec);

        assert!(
            !tp_rsa.contains('='),
            "RSA thumbprint must not contain padding: {tp_rsa}"
        );
        assert!(
            !tp_ec.contains('='),
            "EC thumbprint must not contain padding: {tp_ec}"
        );
    }

    /// EC P-256 thumbprint: result must be a 43-character URL-safe base64
    /// string with no padding and composed only of `[A-Za-z0-9_-]`.
    ///
    /// The x/y values are taken from RFC 7517 §A.1 (EC example public key).
    #[test]
    fn jwk_thumbprint_ec_p256() {
        // RFC 7517 §A.1 EC P-256 key coordinates.
        let jwk = Jwk::Ec {
            crv: "P-256".to_string(),
            x: "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU".to_string(),
            y: "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0".to_string(),
        };

        let thumbprint = jwk_thumbprint(&jwk);

        // SHA-256 produces 32 bytes; base64url-no-pad of 32 bytes is 43 chars.
        assert_eq!(
            thumbprint.len(),
            43,
            "EC thumbprint must be 43 characters (32 bytes SHA-256, base64url no pad); got: {thumbprint}"
        );

        // Must consist solely of URL-safe base64 alphabet.
        assert!(
            thumbprint
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "EC thumbprint must contain only URL-safe base64 chars: {thumbprint}"
        );

        // Must not contain padding.
        assert!(
            !thumbprint.contains('='),
            "EC thumbprint must have no padding: {thumbprint}"
        );

        // Canonical JSON ordering check: crv < kty < x < y.
        let canonical = canonical_ec_json("P-256", &jwk.x_coord(), &jwk.y_coord());
        let crv_pos = canonical.find("\"crv\"").expect("crv member missing");
        let kty_pos = canonical.find("\"kty\"").expect("kty member missing");
        let x_pos = canonical.find("\"x\"").expect("x member missing");
        let y_pos = canonical.find("\"y\"").expect("y member missing");
        assert!(
            crv_pos < kty_pos && kty_pos < x_pos && x_pos < y_pos,
            "Canonical EC JSON must order crv < kty < x < y"
        );
    }

    // Helper trait to access fields for testing.
    impl Jwk {
        fn x_coord(&self) -> String {
            match self {
                Jwk::Ec { x, .. } => x.clone(),
                _ => panic!("not an EC key"),
            }
        }

        fn y_coord(&self) -> String {
            match self {
                Jwk::Ec { y, .. } => y.clone(),
                _ => panic!("not an EC key"),
            }
        }
    }
}
