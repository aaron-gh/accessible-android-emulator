//! Reads which certificates an APK is signed with, from its APK Signing
//! Block, without Java. Google's apksigner needs Java, which most people
//! running AAE don't have, so AAE reads the block itself.
//!
//! This only says whose certificate is in the APK. Android checks the
//! signature itself on installing it, and refuses an APK whose signature
//! doesn't match its certificate, so an APK carrying someone else's
//! certificate can't be installed.
//!
//! The format: the signing block sits just before the ZIP central
//! directory, ending with its size and the magic "APK Sig Block 42". It holds
//! ID-value pairs; the v2 (0x7109871a) and v3 (0xf05368c0) schemes' values
//! each list signers, whose signed data lists their certificates. See
//! <https://source.android.com/docs/security/features/apksigning/v2>.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, IoContext, Result};

const MAGIC: &[u8; 16] = b"APK Sig Block 42";
const V2: u32 = 0x7109_871a;
const V3: u32 = 0xf053_68c0;
const V31: u32 = 0x1b93_ad61;

/// The SHA-256 digests of the certificates an APK is signed with, as
/// lowercase hex, from its v3 or v2 signature.
pub fn certificate_digests(apk: &Path) -> Result<Vec<String>> {
    let bytes = std::fs::read(apk).context(|| format!("Reading {}", apk.display()))?;
    certificates(&bytes)
        .map(|certs| {
            certs
                .iter()
                .map(|cert| hex(&Sha256::digest(cert)))
                .collect()
        })
        .map_err(|reason| Error::Apk {
            path: apk.to_path_buf(),
            reason,
        })
}

/// Every signer's first certificate, from the newest signature scheme present.
fn certificates(apk: &[u8]) -> std::result::Result<Vec<Vec<u8>>, String> {
    let pairs = signing_block(apk)?;
    for id in [V31, V3, V2] {
        if let Some(value) = pairs
            .iter()
            .find(|(pair_id, _)| *pair_id == id)
            .map(|(_, v)| *v)
        {
            let certs =
                signer_certificates(value).ok_or_else(|| "its signature is damaged".to_string())?;
            if !certs.is_empty() {
                return Ok(certs);
            }
        }
    }
    Err("it has no signature AAE can read: it needs one in the v2 or v3 format, which every current build tool makes".into())
}

/// The signing block's ID-value pairs.
fn signing_block(apk: &[u8]) -> std::result::Result<Vec<(u32, &[u8])>, String> {
    let not_signed = || "it isn't signed in the v2 or v3 format".to_string();
    let directory = central_directory_offset(apk).ok_or("it isn't a valid APK")?;
    let footer_start = directory.checked_sub(24).ok_or_else(not_signed)?;
    if apk.get(footer_start + 8..directory) != Some(MAGIC) {
        return Err(not_signed());
    }
    let size = u64_at(apk, footer_start).ok_or_else(not_signed)? as usize;
    // The size counts everything after itself: the pairs, then the footer's
    // copy of the size and the magic. The block starts with it.
    let start = directory
        .checked_sub(size)
        .and_then(|s| s.checked_sub(8))
        .ok_or_else(not_signed)?;
    let mut pairs = Vec::new();
    let mut at = start + 8;
    while at + 12 <= footer_start {
        let length = u64_at(apk, at).ok_or_else(not_signed)? as usize;
        let id = u32_at(apk, at + 8).ok_or_else(not_signed)?;
        let value = apk.get(at + 12..at + 8 + length).ok_or_else(not_signed)?;
        pairs.push((id, value));
        at += 8 + length;
    }
    Ok(pairs)
}

/// Where the ZIP central directory starts, from the end-of-directory record.
fn central_directory_offset(apk: &[u8]) -> Option<usize> {
    // The record is 22 bytes, plus a comment of up to 65535.
    let earliest = apk.len().saturating_sub(22 + 0xFFFF);
    (earliest..=apk.len().checked_sub(22)?)
        .rev()
        .find(|&i| apk[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .and_then(|record| u32_at(apk, record + 16))
        .map(|offset| offset as usize)
}

/// The first certificate of each signer in a v2 or v3 scheme's value.
fn signer_certificates(value: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut certs = Vec::new();
    for signer in sequence(prefixed(value, 0)?)? {
        let signed_data = prefixed(signer, 0)?;
        let digests_length = u32_at(signed_data, 0)? as usize;
        let certificates = prefixed(signed_data, 4 + digests_length)?;
        if let Some(first) = sequence(certificates)?.into_iter().next() {
            certs.push(first.to_vec());
        }
    }
    Some(certs)
}

/// The bytes after a little-endian u32 length at `at`.
fn prefixed(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let length = u32_at(bytes, at)? as usize;
    bytes.get(at + 4..at + 4 + length)
}

/// A run of length-prefixed items.
fn sequence(mut bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let mut items = Vec::new();
    while !bytes.is_empty() {
        let item = prefixed(bytes, 0)?;
        items.push(item);
        bytes = &bytes[4 + item.len()..];
    }
    Some(items)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_length(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    /// A minimal APK: an empty ZIP with a v2 signing block carrying one
    /// signer with the given certificate.
    fn signed_apk(certificate: &[u8], scheme: u32) -> Vec<u8> {
        let digests = with_length(&with_length(b"digest"));
        let certificates = with_length(&with_length(certificate));
        let mut signed_data = digests;
        signed_data.extend(certificates);
        signed_data.extend(with_length(b"")); // additional attributes
        let mut signer = with_length(&signed_data);
        signer.extend(with_length(b"signatures"));
        signer.extend(with_length(b"public key"));
        let value = with_length(&with_length(&signer));

        let mut pair = ((value.len() + 4) as u64).to_le_bytes().to_vec();
        pair.extend(scheme.to_le_bytes());
        pair.extend(&value);
        let size = (pair.len() + 8 + 16) as u64;
        let mut block = size.to_le_bytes().to_vec();
        block.extend(&pair);
        block.extend(size.to_le_bytes());
        block.extend(MAGIC);

        let mut apk = b"local file entries".to_vec();
        apk.extend(&block);
        let directory = apk.len() as u32;
        let mut end = vec![0x50, 0x4b, 0x05, 0x06];
        end.extend([0u8; 12]);
        end.extend(directory.to_le_bytes());
        end.extend([0u8; 2]);
        apk.extend(end);
        apk
    }

    #[test]
    fn reads_the_signing_certificate() {
        for scheme in [V2, V3] {
            let apk = signed_apk(b"a certificate", scheme);
            assert_eq!(certificates(&apk).unwrap(), vec![b"a certificate".to_vec()]);
        }
    }

    #[test]
    fn refuses_apks_without_a_signing_block() {
        let mut apk = b"no block here".to_vec();
        let directory = apk.len() as u32;
        let mut end = vec![0x50, 0x4b, 0x05, 0x06];
        end.extend([0u8; 12]);
        end.extend(directory.to_le_bytes());
        end.extend([0u8; 2]);
        apk.extend(end);
        assert!(certificates(&apk).is_err());
        assert!(certificates(b"not a zip at all").is_err());
    }

    #[test]
    fn reads_a_real_apk_when_one_is_at_hand() {
        // AAE's own helper, if it's been built here: signed by Gradle with
        // v2, so this checks the parser against the real format.
        let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../android/helper/build/outputs/apk/release/helper-release.apk");
        if helper.is_file() {
            let digests = certificate_digests(&helper).unwrap();
            assert_eq!(digests.len(), 1);
            assert_eq!(digests[0].len(), 64);
        }
    }
}

#[cfg(test)]
mod check_any_apk {
    /// AAE_CHECK_APK=<path> cargo test -p aae-core check_any_apk -- --nocapture
    #[test]
    fn prints_an_apks_certificates() {
        if let Some(path) = std::env::var_os("AAE_CHECK_APK") {
            println!(
                "{:?}",
                super::certificate_digests(std::path::Path::new(&path))
            );
        }
    }
}
