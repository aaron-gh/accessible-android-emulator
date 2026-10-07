//! A test client for AAE's remote server, standing in for the Android app.
//!
//!   cargo run -p aae-remote --example client -- pair <host:port> <code>
//!   cargo run -p aae-remote --example client -- call <method> [json params]
//!   cargo run -p aae-remote --example client -- attach <device id> <seconds>
//!   cargo run -p aae-remote --example client -- play <device id> <sound file>
//!   cargo run -p aae-remote --example client -- mic <device id> <seconds>
//!     (sends a 440 Hz tone into the device's microphone)
//!
//! Pairing saves the token and fingerprint in the system's temporary folder.

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

fn saved() -> std::path::PathBuf {
    std::env::temp_dir().join("aae-remote-test-client.json")
}

/// Accepts any certificate, keeping its fingerprint to check.
#[derive(Debug)]
struct Pin(Mutex<Option<String>>);

impl rustls::client::danger::ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        cert: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp: String = Sha256::digest(cert.as_ref())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        *self.0.lock().unwrap() = Some(fp);
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

async fn connect(address: &str) -> Result<(Ws, String)> {
    let pin = Arc::new(Pin(Mutex::new(None)));
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(pin.clone())
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(address).await?;
    tcp.set_nodelay(true)?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(rustls::pki_types::ServerName::try_from("aae.local")?, tcp)
        .await?;
    let (ws, _) = tokio_tungstenite::client_async(format!("wss://{address}/"), tls).await?;
    let fp = pin.0.lock().unwrap().clone().unwrap_or_default();
    Ok((ws, fp))
}

async fn next_json(ws: &mut Ws) -> Result<Value> {
    while let Some(message) = ws.next().await {
        if let Message::Text(text) = message? {
            return Ok(serde_json::from_str(&text)?);
        }
    }
    bail!("the server closed the connection")
}

/// Connects with the saved token.
async fn signed_in() -> Result<Ws> {
    let saved: Value = serde_json::from_slice(&std::fs::read(saved())?)?;
    let (mut ws, fp) = connect(saved["address"].as_str().unwrap()).await?;
    if fp != saved["fingerprint"].as_str().unwrap() {
        bail!("the server's certificate has changed: refusing to connect");
    }
    ws.send(Message::text(
        json!({"type": "hello", "client": saved["client"], "token": saved["token"]}).to_string(),
    ))
    .await?;
    let reply = next_json(&mut ws).await?;
    if reply["type"] != "welcome" {
        bail!("not welcome: {reply}");
    }
    eprintln!("Connected to {}.", reply["server"]);
    Ok(ws)
}

async fn call(ws: &mut Ws, method: &str, params: Value) -> Result<Value> {
    ws.send(Message::text(
        json!({"type": "call", "id": 1, "method": method, "params": params}).to_string(),
    ))
    .await?;
    loop {
        let reply = next_json(ws).await?;
        match reply["type"].as_str() {
            Some("result") => return Ok(reply["result"].clone()),
            Some("error") => bail!("{}", reply["message"]),
            Some("progress") => eprintln!("  {}", reply),
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pair") => {
            let (address, code) = (&args[1], &args[2]);
            let (mut ws, fp) = connect(address).await?;
            let nonce = aae_remote::security::random_text(16);
            let proof = aae_remote::security::client_proof(code, &fp, &nonce);
            ws.send(Message::text(
                json!({"type": "pair", "name": "Test client", "nonce": nonce, "proof": proof})
                    .to_string(),
            ))
            .await?;
            let reply = next_json(&mut ws).await?;
            if reply["type"] != "paired" {
                bail!("not paired: {}", reply["message"]);
            }
            if reply["proof"] != aae_remote::security::server_proof(code, &fp, &nonce) {
                bail!("the server couldn't prove it knows the code");
            }
            std::fs::write(saved(), json!({
                "address": address, "fingerprint": fp, "client": reply["client"], "token": reply["token"],
            }).to_string())?;
            println!("Paired with {}.", reply["server"]);
        }
        Some("call") => {
            let mut ws = signed_in().await?;
            let params = args
                .get(2)
                .map(|p| serde_json::from_str(p))
                .transpose()?
                .unwrap_or(json!({}));
            println!(
                "{}",
                serde_json::to_string_pretty(&call(&mut ws, &args[1], params).await?)?
            );
        }
        Some("attach") => {
            let mut ws = signed_in().await?;
            let size = call(&mut ws, "device.attach", json!({"id": args[1]})).await?;
            println!("Attached: screen {size}");
            let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
            let (mut audio_bytes, mut frames) = (0usize, 0usize);
            while let Ok(Some(message)) = tokio::time::timeout_at(end, ws.next()).await {
                match message? {
                    Message::Binary(bytes) if bytes.first() == Some(&1) => {
                        audio_bytes += bytes.len() - 1;
                        frames += 1;
                    }
                    Message::Text(text) => println!("{text}"),
                    _ => {}
                }
            }
            println!(
                "Sound: {frames} frames, {:.2} seconds of 48 kHz stereo.",
                audio_bytes as f64 / 4.0 / 48_000.0
            );
        }
        Some("play") => {
            // Sends a sound file, then has it played into the microphone.
            let mut ws = signed_in().await?;
            let number = call(&mut ws, "upload.begin", json!({"name": "test.m4a"})).await?;
            let number = number.as_u64().unwrap_or(0) as u32;
            let mut frame = vec![2u8];
            frame.extend_from_slice(&number.to_be_bytes());
            frame.extend_from_slice(&std::fs::read(&args[2])?);
            ws.send(Message::binary(frame)).await?;
            let said = call(
                &mut ws,
                "tools.microphone.play",
                json!({"id": args[1], "upload": number}),
            )
            .await?;
            println!("{said}");
        }
        Some("mic") => {
            let mut ws = signed_in().await?;
            call(&mut ws, "device.attach", json!({"id": args[1]})).await?;
            let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            let (rate, mut phase) = (48_000f32, 0f32);
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(20));
            for _ in 0..seconds * 50 {
                tick.tick().await;
                let mut frame = vec![3u8];
                for _ in 0..960 {
                    phase += 2.0 * std::f32::consts::PI * 440.0 / rate;
                    frame.extend_from_slice(&((phase.sin() * 16_000.0) as i16).to_le_bytes());
                }
                ws.send(Message::binary(frame)).await?;
                // Says what the server says, such as microphone events.
                while let Ok(Some(Ok(Message::Text(text)))) =
                    tokio::time::timeout(std::time::Duration::ZERO, ws.next()).await
                {
                    println!("{text}");
                }
            }
            ws.send(Message::text(
                json!({"type": "microphone", "on": false}).to_string(),
            ))
            .await?;
            println!("Sent {seconds} seconds of tone.");
        }
        _ => bail!(
            "pair <host:port> <code> | call <method> [params] | attach <id> [seconds] | mic <id> [seconds]"
        ),
    }
    Ok(())
}
