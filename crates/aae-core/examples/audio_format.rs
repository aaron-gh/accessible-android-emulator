//! Prints the format the emulator reports for a device's audio packets, and
//! how many samples arrive per second. Usage: audio_format <device>

use std::time::{Duration, Instant};

use aae_core::device::DeviceStore;
use aae_core::{emulator, keys, sdk::Sdk};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let name = std::env::args().nth(1).expect("device name");
    let sdk = Sdk::locate()?;
    let device = DeviceStore::open_default()?.get(&name)?;
    let (_, controller, _) = emulator::attach(&sdk, &device).await?;
    let reader = {
        let controller = controller.clone();
        tokio::spawn(async move {
            let mut stream = controller.stream_audio(48_000, true).await.expect("stream");
            let (mut bytes, mut first) = (0usize, None::<Instant>);
            let mut shown = 0;
            while let Some(Ok(packet)) = stream.next().await {
                if shown < 3 {
                    println!(
                        "packet format: {:?}, {} bytes",
                        packet.format,
                        packet.audio.len()
                    );
                    shown += 1;
                }
                let start = *first.get_or_insert_with(Instant::now);
                bytes += packet.audio.len();
                let secs = start.elapsed().as_secs_f64();
                if secs > 3.0 {
                    println!(
                        "{:.0} bytes per second, which is {:.0} stereo 16-bit frames per second",
                        bytes as f64 / secs,
                        bytes as f64 / secs / 4.0
                    );
                    break;
                }
            }
        })
    };
    for _ in 0..6 {
        controller.press(&keys::parse("volume-up").unwrap()).await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), reader).await;
    Ok(())
}
