//! Who may connect: the server's certificate, the phones paired with it, and
//! pairing codes.
//!
//! TLS with a self-signed certificate. A phone pairs once with a code: its
//! proof is an HMAC of the code over the certificate it received, so a
//! man-in-the-middle certificate fails without the code. The phone then pins
//! the certificate's fingerprint and keeps a token for later connections.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The characters of pairing codes and tokens: no 0, 1, I or O, which are
/// easily confused, so 32 of them, 5 bits each.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
/// Twelve characters: 60 bits.
const CODE_LENGTH: usize = 12;
/// How long a code works for.
pub const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// Wrong tries before a code stops working.
const CODE_TRIES: u32 = 5;

pub fn folder() -> PathBuf {
    aae_core::paths::data_dir().join("remote")
}

/// The server's certificate and private key, in DER, made the first time.
pub struct Identity {
    pub certificate: Vec<u8>,
    pub key: Vec<u8>,
}

impl Identity {
    pub fn load_or_create() -> std::io::Result<Identity> {
        let dir = folder();
        let (cert_path, key_path) = (dir.join("certificate.der"), dir.join("key.der"));
        if let (Ok(certificate), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            return Ok(Identity { certificate, key });
        }
        let made = rcgen::generate_simple_self_signed(vec!["aae.local".to_string()])
            .map_err(std::io::Error::other)?;
        let identity = Identity {
            certificate: made.cert.der().to_vec(),
            key: made.signing_key.serialize_der(),
        };
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&cert_path, &identity.certificate)?;
        write_private(&key_path, &identity.key)?;
        Ok(identity)
    }

    /// The certificate's SHA-256, as lowercase hex: what phones remember.
    pub fn fingerprint(&self) -> String {
        hex(&Sha256::digest(&self.certificate))
    }
}

/// Writes a file only this account can read, where the system allows.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A phone paired with this computer.
#[derive(Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    /// The name the phone gave, such as "Galaxy Z Fold7".
    pub name: String,
    /// The token's SHA-256, so the file alone doesn't let anyone in.
    pub token_hash: String,
    /// When it paired, in seconds since 1970.
    pub paired: u64,
}

/// The paired phones, kept in `clients.json`. Read afresh each time, so a
/// phone unpaired by another of AAE's programs, such as `aae phones
/// --unpair`, is out at once, and isn't written back.
pub struct Clients {
    /// Changes are made one at a time.
    lock: Mutex<()>,
}

impl Clients {
    pub fn load() -> Clients {
        Clients {
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Vec<Client> {
        std::fs::read(folder().join("clients.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, list: &[Client]) {
        let _ = std::fs::create_dir_all(folder());
        if let Ok(json) = serde_json::to_vec_pretty(list) {
            let _ = write_private(&folder().join("clients.json"), &json);
        }
    }

    /// Adds a phone, and returns its token, which only the phone keeps.
    pub fn add(&self, name: &str) -> (Client, String) {
        let token = random_text(32);
        let client = Client {
            id: random_text(8),
            name: name.chars().take(80).collect(),
            token_hash: hex(&Sha256::digest(token.as_bytes())),
            paired: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let _held = self.lock.lock().unwrap();
        let mut list = self.read();
        list.push(client.clone());
        self.save(&list);
        (client, token)
    }

    /// The phone a token belongs to.
    pub fn find(&self, id: &str, token: &str) -> Option<Client> {
        let hash = hex(&Sha256::digest(token.as_bytes()));
        self.read()
            .into_iter()
            .find(|c| c.id == id && equal(c.token_hash.as_bytes(), hash.as_bytes()))
    }

    pub fn list(&self) -> Vec<Client> {
        self.read()
    }

    /// Unpairs a phone. Returns whether it was paired.
    pub fn remove(&self, id: &str) -> bool {
        let _held = self.lock.lock().unwrap();
        let mut list = self.read();
        let before = list.len();
        list.retain(|c| c.id != id);
        let removed = list.len() != before;
        if removed {
            self.save(&list);
        }
        removed
    }
}

/// The pairing code being offered, if any. It's kept in a file in AAE's
/// private data folder, so a code made by any of AAE's programs, such as
/// `aae pair` or the apps, works with whichever server is running, such as
/// one started at login.
#[derive(Default)]
pub struct Pairing {
    /// Codes are checked one at a time.
    lock: Mutex<()>,
}

#[derive(Serialize, Deserialize)]
struct Code {
    text: String,
    /// When it was made, in seconds since 1970.
    made: u64,
    tries: u32,
}

fn code_path() -> PathBuf {
    folder().join("pairing.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn save_code(code: Option<&Code>) {
    match code {
        Some(code) => {
            let _ = std::fs::create_dir_all(folder());
            if let Ok(json) = serde_json::to_vec(code) {
                let _ = write_private(&code_path(), &json);
            }
        }
        None => {
            let _ = std::fs::remove_file(code_path());
        }
    }
}

impl Pairing {
    /// Makes a new code, replacing any other. Returns it as shown to people:
    /// "ABCD-EFGH-JKLM".
    pub fn new_code(&self) -> String {
        let _held = self.lock.lock().unwrap();
        let text = random_text(CODE_LENGTH);
        save_code(Some(&Code {
            text: text.clone(),
            made: now(),
            tries: 0,
        }));
        show_code(&text)
    }

    /// Checks a phone's proof that it knows the code, for a certificate
    /// fingerprint and the phone's nonce. A right proof uses the code up, so
    /// it pairs one phone. Returns the code, to prove the server knows it too.
    pub fn check(
        &self,
        fingerprint: &str,
        nonce: &str,
        proof: &str,
    ) -> Result<String, &'static str> {
        let _held = self.lock.lock().unwrap();
        let code: Option<Code> = std::fs::read(code_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let Some(mut code) = code else {
            return Err(
                "This computer isn't offering a pairing code. Make a new one on it, then try again.",
            );
        };
        if now().saturating_sub(code.made) > CODE_LIFETIME.as_secs() {
            save_code(None);
            return Err("The pairing code has run out. Make a new one on the computer.");
        }
        if equal(
            proof.as_bytes(),
            client_proof(&code.text, fingerprint, nonce).as_bytes(),
        ) {
            save_code(None);
            return Ok(code.text);
        }
        code.tries += 1;
        if code.tries >= CODE_TRIES {
            save_code(None);
            return Err(
                "That code was wrong too many times, so it no longer works. Make a new one on the computer.",
            );
        }
        save_code(Some(&code));
        Err("That pairing code isn't right. Check it on the computer, and try again.")
    }
}

/// Codes are typed in any case, with or without dashes and spaces.
pub fn normalise_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn show_code(text: &str) -> String {
    text.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

fn hmac(code: &str, parts: &[&str]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(normalise_code(code).as_bytes())
        .expect("HMAC takes keys of any length");
    for part in parts {
        mac.update(part.as_bytes());
        mac.update(b"\n");
    }
    hex(&mac.finalize().into_bytes())
}

/// What the phone sends to prove it knows the code.
pub fn client_proof(code: &str, fingerprint: &str, nonce: &str) -> String {
    hmac(code, &["aae-pair-1 phone", fingerprint, nonce])
}

/// What the server sends back to prove it knows the code too.
pub fn server_proof(code: &str, fingerprint: &str, nonce: &str) -> String {
    hmac(code, &["aae-pair-1 computer", fingerprint, nonce])
}

/// Random text from [`ALPHABET`].
pub fn random_text(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    getrandom::fill(&mut bytes).expect("the system's random numbers");
    bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect()
}

/// Compares in constant time, so timing doesn't give secrets away.
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests share the code file, so they use their own folder, one at a time.
    fn isolated() -> std::sync::MutexGuard<'static, ()> {
        static ONE: Mutex<()> = Mutex::new(());
        let guard = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("aae-pairing-test-{}", std::process::id()));
        // SAFETY: the tests that read it hold the lock.
        unsafe { std::env::set_var("AAE_HOME", &dir) };
        guard
    }

    #[test]
    fn a_right_proof_pairs_once() {
        let _one = isolated();
        let pairing = Pairing::default();
        let shown = pairing.new_code();
        assert_eq!(shown.len(), 14);
        let typed = shown.to_lowercase().replace('-', " ");
        let proof = client_proof(&typed, "fp", "nonce");
        assert!(pairing.check("fp", "nonce", &proof).is_ok());
        // Used up.
        assert!(pairing.check("fp", "nonce", &proof).is_err());
    }

    #[test]
    fn a_proof_for_another_certificate_fails() {
        let _one = isolated();
        let pairing = Pairing::default();
        let code = pairing.new_code();
        // A phone shown someone else's certificate proves over that one.
        let proof = client_proof(&code, "someone else's", "nonce");
        assert!(pairing.check("ours", "nonce", &proof).is_err());
    }

    #[test]
    fn too_many_wrong_tries_end_the_code() {
        let _one = isolated();
        let pairing = Pairing::default();
        let code = pairing.new_code();
        for _ in 0..CODE_TRIES {
            assert!(pairing.check("fp", "n", "wrong").is_err());
        }
        let proof = client_proof(&code, "fp", "n");
        assert!(pairing.check("fp", "n", &proof).is_err());
    }

    #[test]
    fn codes_are_twelve_characters_of_the_alphabet() {
        let _one = isolated();
        let code = normalise_code(&Pairing::default().new_code());
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
    }
}
