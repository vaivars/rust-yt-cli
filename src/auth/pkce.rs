use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};

pub struct Pkce {
    pub verifier: SecretString,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_token(32);
        Self {
            challenge: challenge_for(&verifier),
            verifier: SecretString::from(verifier),
        }
    }
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `bytes` random bytes, base64url-encoded without padding.
pub fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::fill(buffer.as_mut_slice());
    URL_SAFE_NO_PAD.encode(buffer)
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn rfc_7636_example() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifiers_are_long_random_and_match_their_challenge() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.verifier.expose_secret().len(), 43);
        assert_ne!(a.verifier.expose_secret(), b.verifier.expose_secret());
        assert_eq!(a.challenge, challenge_for(a.verifier.expose_secret()));
    }
}
