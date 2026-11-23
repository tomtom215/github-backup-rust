// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub App authentication: the private key, the JWT, and the cached
//! installation access token.
//!
//! The flow (GitHub Docs, "Generating a JSON Web Token (JWT) for a GitHub
//! App", "Generating an installation access token for a GitHub App"):
//!
//! 1. A JWT signed with the App's private key (RS256) authenticates as the
//!    App.  Claims: `iat` 60 seconds in the past (clock drift), `exp` at most
//!    10 minutes ahead, `iss` the App's client ID (recommended) or App ID.
//! 2. `POST /app/installations/{id}/access_tokens` with that JWT returns an
//!    installation access token (`ghs_...`) that expires after one hour.
//! 3. The installation token is used like any other token, for the REST API
//!    and as the HTTP password of git (user name `x-access-token`).
//!
//! # Cryptography
//!
//! RS256 (RSASSA-PKCS1-v1_5 with SHA-256) comes from `aws-lc-rs`, never from
//! code of our own.  It is the provider rustls already uses for every TLS
//! connection of this program, so it adds no crate to the build and no new
//! code to trust.  The pure-Rust `rsa` crate was the alternative: its Marvin
//! timing side channel (RUSTSEC-2023-0071) is fixed in 0.10, but that is a
//! new major version built on a new big-integer library, and it would add
//! about ten crates to the supply chain for the same operation.  (Marvin
//! matters most for decryption; this module only signs.)
//!
//! The PEM text and the decoded key bytes this module handles are wiped from
//! memory when dropped (`zeroize`); the parsed key lives inside aws-lc, whose
//! allocator clears memory when it is freed.

use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::signature::RSA_PKCS1_SHA256;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::Instant;
use zeroize::Zeroizing;

use github_backup_types::config::AppCredential;

use crate::error::ClientError;

/// An installation token is renewed when less than this is left of it.
pub(crate) const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// GitHub's lifetime of an installation token.  A token is never trusted for
/// longer, whatever `expires_at` says against a local clock that may be slow.
const TOKEN_LIFETIME: Duration = Duration::from_secs(60 * 60);
/// `iat` is set this far in the past, as GitHub recommends against clock drift.
const JWT_BACKDATE_SECS: u64 = 60;
/// `exp` is set this far ahead: under GitHub's limit of 10 minutes, with a
/// minute to spare for a local clock that runs fast.
const JWT_LIFETIME_SECS: u64 = 9 * 60;

/// The App's RSA private key.
pub(crate) struct AppKey(KeyPair);

impl AppKey {
    /// Reads a PEM private key: PKCS#1 (`BEGIN RSA PRIVATE KEY`, the format
    /// GitHub issues) or unencrypted PKCS#8 (`BEGIN PRIVATE KEY`).
    ///
    /// # Errors
    ///
    /// [`ClientError::App`] naming what is wrong; never any key material.
    pub(crate) fn from_pem(pem: &str) -> Result<Self, ClientError> {
        let bad = |why: &str| ClientError::App(format!("the private key cannot be used: {why}"));
        let (label, body) = pem_block(pem)
            .ok_or_else(|| bad("no PEM block found (expected -----BEGIN RSA PRIVATE KEY-----)"))?;
        // A legacy encrypted PEM has `Proc-Type:` / `DEK-Info:` header lines.
        if label == "ENCRYPTED PRIVATE KEY" || body.lines().any(|l| l.contains(':')) {
            return Err(bad(
                "it is encrypted; use the unencrypted .pem file GitHub issued",
            ));
        }
        let mut base64 = Zeroizing::new(String::with_capacity(body.len()));
        for line in body.lines() {
            base64.push_str(line.trim());
        }
        let der = Zeroizing::new(
            STANDARD
                .decode(base64.as_bytes())
                .map_err(|_| bad("the PEM body is not valid base64"))?,
        );
        let key = match label {
            "RSA PRIVATE KEY" => KeyPair::from_der(&der),
            "PRIVATE KEY" => KeyPair::from_pkcs8(&der),
            other => return Err(bad(&format!("a {other} block is not an RSA private key"))),
        }
        .map_err(|e| bad(&format!("rejected as an RSA key ({e})")))?;
        Ok(Self(key))
    }
}

/// The label and body of the first PEM block in `text`.
fn pem_block(text: &str) -> Option<(&str, &str)> {
    let start = text.find("-----BEGIN ")? + "-----BEGIN ".len();
    let (label, rest) = text[start..].split_once("-----")?;
    let end = rest.find(&format!("-----END {label}-----"))?;
    Some((label, &rest[..end]))
}

/// A JWT for the App `issuer`, valid from a minute before `now_unix` (seconds
/// since the epoch) for [`JWT_LIFETIME_SECS`].
///
/// A numeric issuer (an App ID) is sent as a JSON number, a client ID as a
/// string.
///
/// # Errors
///
/// [`ClientError::App`] if signing fails.
pub(crate) fn jwt(
    issuer: &str,
    key: &AppKey,
    now_unix: u64,
) -> Result<Zeroizing<String>, ClientError> {
    let iss = issuer
        .parse::<u64>()
        .map_or_else(|_| json!(issuer), |id| json!(id));
    let claims = json!({
        "iat": now_unix.saturating_sub(JWT_BACKDATE_SECS),
        "exp": now_unix + JWT_LIFETIME_SECS,
        "iss": iss,
    });
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut signature = vec![0; key.0.public_modulus_len()];
    key.0
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .map_err(|_| ClientError::App("signing the JWT failed".into()))?;
    Ok(Zeroizing::new(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    )))
}

/// The state of a GitHub App credential, shared by every clone of a client.
pub(crate) struct AppAuth {
    issuer: String,
    key: AppKey,
    /// The installation tokens are minted for.
    pub(crate) installation_id: Option<u64>,
    /// The current installation token.  The lock is held while a new one is
    /// minted, so concurrent callers wait for that one instead of each
    /// minting their own.
    pub(crate) current: Mutex<Option<Minted>>,
}

impl AppAuth {
    /// Parses the key of `credential`.
    ///
    /// # Errors
    ///
    /// [`ClientError::App`] for an empty App ID or an unusable key.
    pub(crate) fn new(credential: &AppCredential) -> Result<Self, ClientError> {
        let issuer = credential.app_id.trim();
        if issuer.is_empty() {
            return Err(ClientError::App("the App ID / client ID is empty".into()));
        }
        Ok(Self {
            issuer: issuer.to_string(),
            key: AppKey::from_pem(&credential.private_key_pem)?,
            installation_id: credential.installation_id,
            current: Mutex::new(None),
        })
    }

    /// A JWT for this App, valid now.
    ///
    /// # Errors
    ///
    /// See [`jwt`].
    pub(crate) fn jwt(&self) -> Result<Zeroizing<String>, ClientError> {
        jwt(&self.issuer, &self.key, crate::client::unix_now())
    }
}

/// An installation token and when to replace it.
pub(crate) struct Minted {
    pub(crate) token: Zeroizing<String>,
    renew_at: Instant,
}

impl Minted {
    /// Reads the answer to `POST /app/installations/{id}/access_tokens`,
    /// requested at `requested_at` (`now_unix` by the wall clock).
    ///
    /// The token is renewed [`REFRESH_MARGIN`] before `expires_at`, measured
    /// from the request (so that a slow answer counts against the token), and
    /// at most [`TOKEN_LIFETIME`] after it.
    ///
    /// # Errors
    ///
    /// [`ClientError::App`] when the answer has no token.
    pub(crate) fn from_response(
        body: &[u8],
        requested_at: Instant,
        now_unix: u64,
    ) -> Result<Self, ClientError> {
        let value: Value = serde_json::from_slice(body)?;
        let token = value["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ClientError::App("GitHub returned no installation token".into()))?;
        let lifetime = value["expires_at"]
            .as_str()
            .and_then(|at| at.parse::<jiff::Timestamp>().ok())
            .and_then(|at| u64::try_from(at.as_second()).ok())
            .map_or(TOKEN_LIFETIME, |at| {
                Duration::from_secs(at.saturating_sub(now_unix)).min(TOKEN_LIFETIME)
            });
        Ok(Self {
            token: Zeroizing::new(token.to_string()),
            renew_at: requested_at + lifetime.saturating_sub(REFRESH_MARGIN),
        })
    }

    /// Whether the token may still be used (it has more than
    /// [`REFRESH_MARGIN`] left).
    pub(crate) fn is_fresh(&self) -> bool {
        Instant::now() < self.renew_at
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use aws_lc_rs::encoding::AsDer as _;
    use aws_lc_rs::rsa::KeySize;
    use aws_lc_rs::signature::{KeyPair as _, UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};

    /// A fresh 2048-bit key, generated for the test run (no key is committed),
    /// as `(PKCS#1 PEM, PKCS#8 PEM, public key DER)`.
    pub(crate) fn test_key() -> (String, String, Vec<u8>) {
        let pair = KeyPair::generate(KeySize::Rsa2048).expect("generate");
        let pkcs8 = pair.as_der().expect("pkcs8").as_ref().to_vec();
        let pkcs1 = pkcs1_from_pkcs8(&pkcs8);
        let pem = |label: &str, der: &[u8]| {
            let body = STANDARD.encode(der);
            let lines: Vec<&str> = body
                .as_bytes()
                .chunks(64)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect();
            format!(
                "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
                lines.join("\n")
            )
        };
        (
            pem("RSA PRIVATE KEY", &pkcs1),
            pem("PRIVATE KEY", &pkcs8),
            pair.public_key().as_ref().to_vec(),
        )
    }

    /// The `RSAPrivateKey` inside a PKCS#8 `PrivateKeyInfo`: the content of
    /// its OCTET STRING, the third element of the outer SEQUENCE.
    fn pkcs1_from_pkcs8(der: &[u8]) -> Vec<u8> {
        // (tag, content) of the DER element at the start of `d`, and the rest.
        fn element(d: &[u8]) -> (u8, &[u8], &[u8]) {
            let (tag, first) = (d[0], d[1]);
            let (len, header) = if first < 0x80 {
                (usize::from(first), 2)
            } else {
                let n = usize::from(first & 0x7f);
                let len = d[2..2 + n]
                    .iter()
                    .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
                (len, 2 + n)
            };
            (tag, &d[header..header + len], &d[header + len..])
        }
        let (_, info, _) = element(der);
        let (_, _version, rest) = element(info);
        let (_, _algorithm, rest) = element(rest);
        let (tag, key, _) = element(rest);
        assert_eq!(tag, 0x04, "OCTET STRING");
        key.to_vec()
    }

    fn decode_part(part: &str) -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).expect("base64url")).expect("json")
    }

    #[test]
    fn the_jwt_has_githubs_header_and_claims_and_verifies_with_the_public_key() {
        let (pkcs1, _, public) = test_key();
        let key = AppKey::from_pem(&pkcs1).expect("PKCS#1 key");
        let token = jwt("Iv23liClientId", &key, 1_700_000_000).expect("jwt");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(decode_part(parts[0]), json!({"alg": "RS256", "typ": "JWT"}));
        assert_eq!(
            decode_part(parts[1]),
            json!({"iat": 1_700_000_000 - 60, "exp": 1_700_000_000 + 540, "iss": "Iv23liClientId"})
        );
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).expect("signature");
        let signed = format!("{}.{}", parts[0], parts[1]);
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, &public)
            .verify(signed.as_bytes(), &signature)
            .expect("the signature verifies with the App's public key");
        // Any change to the signed part breaks it.
        let tampered = format!("{}.{}x", parts[0], parts[1]);
        assert!(UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, &public)
            .verify(tampered.as_bytes(), &signature)
            .is_err());
    }

    #[test]
    fn a_numeric_app_id_is_sent_as_a_number_and_exp_is_within_ten_minutes() {
        let (_, pkcs8, _) = test_key();
        let key = AppKey::from_pem(&pkcs8).expect("PKCS#8 key");
        let token = jwt("123456", &key, 1000).expect("jwt");
        let claims = decode_part(token.split('.').nth(1).unwrap());
        assert_eq!(claims["iss"], json!(123_456));
        let (iat, exp) = (
            claims["iat"].as_u64().unwrap(),
            claims["exp"].as_u64().unwrap(),
        );
        assert!(iat < 1000 && exp > 1000 && exp - 1000 <= 600, "{claims}");
    }

    #[test]
    fn unusable_keys_are_refused_without_showing_them() {
        for (pem, says) in [
            ("not a key at all", "no PEM block"),
            (
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nQUJD\n-----END ENCRYPTED PRIVATE KEY-----",
                "encrypted",
            ),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nQUJD\n-----END RSA PRIVATE KEY-----",
                "encrypted",
            ),
            (
                "-----BEGIN RSA PRIVATE KEY-----\n!!!!\n-----END RSA PRIVATE KEY-----",
                "base64",
            ),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nQUJDREVGR0g=\n-----END RSA PRIVATE KEY-----",
                "rejected",
            ),
            (
                "-----BEGIN EC PRIVATE KEY-----\nQUJD\n-----END EC PRIVATE KEY-----",
                "not an RSA private key",
            ),
        ] {
            let e = AppKey::from_pem(pem).err().expect("refused").to_string();
            assert!(e.contains(says), "{pem}: {e}");
            assert!(!e.contains("QUJD"), "{e}");
        }
    }

    #[tokio::test]
    async fn a_token_is_renewed_five_minutes_before_it_expires() {
        let now = 1_700_000_000;
        let at = |secs: u64| {
            let ts = jiff::Timestamp::from_second(i64::try_from(now + secs).unwrap()).unwrap();
            format!(r#"{{"token":"ghs_x","expires_at":"{ts}"}}"#)
        };
        let fresh = Minted::from_response(at(3600).as_bytes(), Instant::now(), now).unwrap();
        assert!(fresh.is_fresh());
        assert_eq!(&*fresh.token, "ghs_x");
        // Four minutes left: inside the margin, so renewed at once.
        let ending = Minted::from_response(at(240).as_bytes(), Instant::now(), now).unwrap();
        assert!(!ending.is_fresh());
        // A local clock hours behind GitHub's: still no more than an hour.
        let slow = Minted::from_response(at(5 * 3600).as_bytes(), Instant::now(), now).unwrap();
        assert!(slow.renew_at <= Instant::now() + TOKEN_LIFETIME - REFRESH_MARGIN);
        assert!(Minted::from_response(br#"{"expires_at":"x"}"#, Instant::now(), now).is_err());
    }
}
