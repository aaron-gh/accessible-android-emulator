//! A SPICE client for a Googlebook device's sound.
//!
//! QEMU sends the VM's audio output to SPICE clients, as signed 16-bit
//! samples. AAE connects to the VM's SPICE socket as such a client and plays
//! the sound itself, so volume, mute and output choice work as they do for the
//! emulator. Only the main channel (which SPICE needs before any other) and
//! the playback channel are used.

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use num_bigint::BigUint;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::error::{Error, Result};

const MAGIC: u32 = u32::from_le_bytes(*b"REDQ");
const CHANNEL_MAIN: u8 = 1;
const CHANNEL_PLAYBACK: u8 = 5;
const CAP_AUTH_SPICE: u32 = 1 << 1;
const CAP_MINI_HEADER: u32 = 1 << 3;

const MSG_SET_ACK: u16 = 3;
const MSG_PING: u16 = 4;
const MSG_MAIN_INIT: u16 = 103;
const MSG_PLAYBACK_DATA: u16 = 101;
const MSG_PLAYBACK_MODE: u16 = 102;
const MSG_PLAYBACK_START: u16 = 103;
const MSGC_ACK_SYNC: u16 = 1;
const MSGC_ACK: u16 = 2;
const MSGC_PONG: u16 = 3;
const AUDIO_MODE_RAW: u16 = 1;

/// One block of the device's sound: interleaved stereo samples, at the rate
/// asked for in [`playback`].
pub struct Block {
    /// When it arrived, in microseconds since 1970.
    pub timestamp: u64,
    pub samples: Vec<i16>,
}

/// Starts receiving the device's sound from the SPICE socket at `path`,
/// resampled to `rate`. Ends when the receiver is dropped or the VM stops.
pub async fn playback(path: &Path, rate: u32) -> Result<mpsc::Receiver<Block>> {
    let mut main = Channel::link(path, 0, CHANNEL_MAIN).await?;
    let session = loop {
        let (kind, body) = main.read().await?;
        if kind == MSG_MAIN_INIT && body.len() >= 4 {
            break u32::from_le_bytes(body[0..4].try_into().unwrap());
        }
        main.common(kind, &body).await?;
    };
    let mut sound = Channel::link(path, session, CHANNEL_PLAYBACK).await?;
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        // The main channel has to stay connected and answered, or the server
        // drops the session.
        let keep_main = tokio::spawn(async move {
            while let Ok((kind, body)) = main.read().await {
                if main.common(kind, &body).await.is_err() {
                    break;
                }
            }
        });
        let mut source_rate = 48_000;
        let mut resampler = crate::audio::Resampler::new(source_rate, rate);
        loop {
            let (kind, body) = match sound.read().await {
                Ok(message) => message,
                Err(e) => {
                    tracing::debug!("VM sound channel ended: {e}");
                    break;
                }
            };
            match kind {
                MSG_PLAYBACK_START if body.len() >= 10 => {
                    let frequency = u32::from_le_bytes(body[6..10].try_into().unwrap());
                    if frequency != source_rate && frequency > 0 {
                        source_rate = frequency;
                        resampler = crate::audio::Resampler::new(source_rate, rate);
                    }
                }
                MSG_PLAYBACK_MODE if body.len() >= 6 => {
                    let mode = u16::from_le_bytes([body[4], body[5]]);
                    if mode != AUDIO_MODE_RAW {
                        tracing::warn!("VM sound arrives encoded ({mode}), which AAE can't play");
                    }
                }
                MSG_PLAYBACK_DATA if body.len() > 4 => {
                    let mut samples = Vec::with_capacity(body.len() / 2);
                    for frame in body[4..].chunks_exact(4) {
                        let left = i16::from_le_bytes([frame[0], frame[1]]);
                        let right = i16::from_le_bytes([frame[2], frame[3]]);
                        resampler.push(left, right, &mut samples);
                    }
                    let block = Block {
                        timestamp: now_micros(),
                        samples,
                    };
                    if sender.send(block).await.is_err() {
                        break;
                    }
                }
                _ => {
                    if sound.common(kind, &body).await.is_err() {
                        break;
                    }
                }
            }
        }
        keep_main.abort();
    });
    Ok(receiver)
}

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default()
}

/// One linked SPICE channel.
struct Channel {
    stream: UnixStream,
    mini_header: bool,
    /// Acknowledge every this many messages (0: not asked to).
    ack_window: u32,
    unacked: u32,
    serial: u64,
}

impl Channel {
    async fn link(path: &Path, session: u32, kind: u8) -> Result<Channel> {
        let fail = |what: &str| Error::Vm(format!("The VM's sound connection {what}"));
        let mut stream = tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(path))
            .await
            .map_err(|_| fail("didn't open"))?
            .map_err(|e| Error::Vm(format!("The VM's sound connection: {e}")))?;

        // Link message: header, then the message and one word of common capabilities.
        let mut body = Vec::new();
        body.extend_from_slice(&session.to_le_bytes());
        body.push(kind);
        body.push(0);
        body.extend_from_slice(&1u32.to_le_bytes()); // common capability words
        body.extend_from_slice(&0u32.to_le_bytes()); // channel capability words
        body.extend_from_slice(&18u32.to_le_bytes()); // capabilities follow the message
        body.extend_from_slice(&(CAP_AUTH_SPICE | CAP_MINI_HEADER).to_le_bytes());
        let mut link = Vec::new();
        for word in [MAGIC, 2, 2, body.len() as u32] {
            link.extend_from_slice(&word.to_le_bytes());
        }
        link.extend_from_slice(&body);
        stream.write_all(&link).await.map_err(io)?;

        let mut header = [0u8; 16];
        stream.read_exact(&mut header).await.map_err(io)?;
        if header[0..4] != MAGIC.to_le_bytes() {
            return Err(fail("got an answer that isn't SPICE"));
        }
        let size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        if !(178..=4096).contains(&size) {
            return Err(fail("got a malformed answer"));
        }
        let mut reply = vec![0u8; size];
        stream.read_exact(&mut reply).await.map_err(io)?;
        let error = u32::from_le_bytes(reply[0..4].try_into().unwrap());
        if error != 0 {
            return Err(Error::Vm(format!(
                "SPICE refused the connection (error {error})"
            )));
        }
        let key = &reply[4..166];
        let common_words = u32::from_le_bytes(reply[166..170].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(reply[174..178].try_into().unwrap()) as usize;
        let server_caps = reply
            .get(offset..offset + 4)
            .filter(|_| common_words > 0)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .unwrap_or(0);

        // The password is empty: AAE's VMs turn ticketing off, but the server
        // still expects one, encrypted with its key.
        let ticket = encrypt_ticket(key).ok_or_else(|| fail("got a key it can't use"))?;
        stream.write_all(&ticket).await.map_err(io)?;
        let mut result = [0u8; 4];
        stream.read_exact(&mut result).await.map_err(io)?;
        let result = u32::from_le_bytes(result);
        if result != 0 {
            return Err(Error::Vm(format!(
                "SPICE refused the connection (error {result})"
            )));
        }
        Ok(Channel {
            stream,
            mini_header: server_caps & CAP_MINI_HEADER != 0,
            ack_window: 0,
            unacked: 0,
            serial: 0,
        })
    }

    async fn read(&mut self) -> Result<(u16, Vec<u8>)> {
        let (kind, size) = if self.mini_header {
            let mut header = [0u8; 6];
            self.stream.read_exact(&mut header).await.map_err(io)?;
            (
                u16::from_le_bytes([header[0], header[1]]),
                u32::from_le_bytes(header[2..6].try_into().unwrap()),
            )
        } else {
            let mut header = [0u8; 18];
            self.stream.read_exact(&mut header).await.map_err(io)?;
            (
                u16::from_le_bytes([header[8], header[9]]),
                u32::from_le_bytes(header[10..14].try_into().unwrap()),
            )
        };
        if size > 16 << 20 {
            return Err(Error::Vm(
                "The VM's sound connection sent too much at once".into(),
            ));
        }
        let mut body = vec![0u8; size as usize];
        self.stream.read_exact(&mut body).await.map_err(io)?;
        if self.ack_window > 0 {
            self.unacked += 1;
            if self.unacked >= self.ack_window {
                self.unacked = 0;
                self.write(MSGC_ACK, &[]).await?;
            }
        }
        Ok((kind, body))
    }

    async fn write(&mut self, kind: u16, body: &[u8]) -> Result<()> {
        let mut message = Vec::with_capacity(18 + body.len());
        if self.mini_header {
            message.extend_from_slice(&kind.to_le_bytes());
            message.extend_from_slice(&(body.len() as u32).to_le_bytes());
        } else {
            self.serial += 1;
            message.extend_from_slice(&self.serial.to_le_bytes());
            message.extend_from_slice(&kind.to_le_bytes());
            message.extend_from_slice(&(body.len() as u32).to_le_bytes());
            message.extend_from_slice(&0u32.to_le_bytes());
        }
        message.extend_from_slice(body);
        self.stream.write_all(&message).await.map_err(io)
    }

    /// Answers the messages every channel gets.
    async fn common(&mut self, kind: u16, body: &[u8]) -> Result<()> {
        match kind {
            MSG_SET_ACK if body.len() >= 8 => {
                self.ack_window = u32::from_le_bytes(body[4..8].try_into().unwrap());
                self.unacked = 0;
                self.write(MSGC_ACK_SYNC, &body[0..4]).await
            }
            MSG_PING if body.len() >= 12 => self.write(MSGC_PONG, &body[0..12]).await,
            _ => Ok(()),
        }
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Vm(format!("The VM's sound connection: {e}"))
}

/// The empty password, encrypted with RSA-OAEP (SHA-1) under the server's
/// 1024-bit public key, given as DER SubjectPublicKeyInfo.
fn encrypt_ticket(der: &[u8]) -> Option<Vec<u8>> {
    let (modulus, exponent) = rsa_key(der)?;
    let n = BigUint::from_bytes_be(modulus);
    let e = BigUint::from_bytes_be(exponent);
    let k = modulus.iter().skip_while(|&&b| b == 0).count();
    if k != 128 {
        return None;
    }
    let message = [0u8]; // the password "" with its terminating NUL
    let hash_len = 20;
    let mut db = vec![0u8; k - hash_len - 1];
    db[..hash_len].copy_from_slice(&Sha1::digest([]));
    let one = db.len() - message.len() - 1;
    db[one] = 1;
    db[one + 1..].copy_from_slice(&message);
    let seed = random_bytes(hash_len);
    let masked_db: Vec<u8> = db
        .iter()
        .zip(mgf1(&seed, db.len()))
        .map(|(a, b)| a ^ b)
        .collect();
    let masked_seed: Vec<u8> = seed
        .iter()
        .zip(mgf1(&masked_db, hash_len))
        .map(|(a, b)| a ^ b)
        .collect();
    let mut encoded = vec![0u8];
    encoded.extend_from_slice(&masked_seed);
    encoded.extend_from_slice(&masked_db);
    let c = BigUint::from_bytes_be(&encoded)
        .modpow(&e, &n)
        .to_bytes_be();
    let mut out = vec![0u8; k - c.len()];
    out.extend_from_slice(&c);
    Some(out)
}

fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 20);
    let mut counter = 0u32;
    while out.len() < len {
        let mut hasher = Sha1::new();
        hasher.update(seed);
        hasher.update(counter.to_be_bytes());
        out.extend_from_slice(&hasher.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

fn random_bytes(len: usize) -> Vec<u8> {
    let state = std::collections::hash_map::RandomState::new();
    let mut out = Vec::with_capacity(len + 8);
    let mut i = 0u64;
    while out.len() < len {
        let mut hasher = state.build_hasher();
        hasher.write_u64(i);
        hasher.write_u128(now_micros() as u128);
        out.extend_from_slice(&hasher.finish().to_le_bytes());
        i += 1;
    }
    out.truncate(len);
    out
}

/// The modulus and exponent of an RSA SubjectPublicKeyInfo.
fn rsa_key(der: &[u8]) -> Option<(&[u8], &[u8])> {
    // SEQUENCE { SEQUENCE { algorithm }, BIT STRING { SEQUENCE { INTEGER n, INTEGER e } } }
    let (_, info, _) = der_item(der)?;
    let (_, _, rest) = der_item(info)?;
    let (tag, bits, _) = der_item(rest)?;
    if tag != 0x03 || bits.first() != Some(&0) {
        return None;
    }
    let (_, key, _) = der_item(&bits[1..])?;
    let (tag_n, n, rest) = der_item(key)?;
    let (tag_e, e, _) = der_item(rest)?;
    (tag_n == 0x02 && tag_e == 0x02).then_some((n, e))
}

/// One DER item: its tag, its contents and what follows it.
fn der_item(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *data.first()?;
    let first = *data.get(1)? as usize;
    let (len, start) = if first < 0x80 {
        (first, 2)
    } else {
        let count = first & 0x7f;
        if count == 0 || count > 4 {
            return None;
        }
        let len = data
            .get(2..2 + count)?
            .iter()
            .fold(0usize, |acc, &b| acc << 8 | b as usize);
        (len, 2 + count)
    };
    let contents = data.get(start..start + len)?;
    Some((tag, contents, &data[start + len..]))
}

/// Where a running Googlebook device's SPICE socket is.
pub fn socket(run_dir: &Path) -> PathBuf {
    run_dir.join("spice.sock")
}
