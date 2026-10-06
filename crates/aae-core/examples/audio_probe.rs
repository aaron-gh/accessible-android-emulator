//! Prints the loudest sample in each quarter second of a running device's audio
//! while pressing keys on it, to see what the emulator's audio stream carries.
//! Usage: audio_probe <device> [key ...]

use std::time::{Duration, Instant};

use aae_core::device::DeviceStore;
use aae_core::{emulator, keys, sdk::Sdk};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let name = args.next().expect("device name");
    let mut presses: Vec<String> = args.collect();
    if presses.is_empty() {
        presses = ["meta+right", "meta+right", "meta+left"]
            .map(String::from)
            .to_vec();
    }
    let sdk = Sdk::locate()?;
    let device = DeviceStore::open_default()?.get(&name)?;
    let (_, controller, _) = emulator::attach(&sdk, &device).await?;
    let start = Instant::now();

    let reader = {
        let controller = controller.clone();
        tokio::spawn(async move {
            let mut stream = controller.stream_audio(48_000, true).await.expect("stream");
            let mut window_end = start.elapsed().as_millis() / 250 * 250 + 250;
            let (mut peak, mut sum_sq, mut n) = (0u16, 0f64, 0u64);
            while let Some(packet) = stream.next().await {
                for s in packet.expect("packet").audio.chunks_exact(2) {
                    let v = i16::from_le_bytes([s[0], s[1]]);
                    peak = peak.max(v.unsigned_abs());
                    sum_sq += (v as f64) * (v as f64);
                    n += 1;
                }
                let now = start.elapsed().as_millis();
                if now >= window_end {
                    let rms = if n > 0 {
                        (sum_sq / n as f64).sqrt()
                    } else {
                        0.0
                    };
                    println!("{window_end:>6} ms: peak {peak:>5}, rms {rms:>7.1}");
                    window_end = now / 250 * 250 + 250;
                    (peak, sum_sq, n) = (0, 0.0, 0);
                }
            }
        })
    };

    for key in &presses {
        tokio::time::sleep(Duration::from_millis(6000)).await;
        println!("{:>6} ms: pressing {key}", start.elapsed().as_millis());
        controller
            .press(&keys::parse(key).expect("key name"))
            .await?;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    reader.abort();
    Ok(())
}
