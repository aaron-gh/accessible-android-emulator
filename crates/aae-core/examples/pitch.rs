//! Plays a 1,000 Hz test tone on a device through AAE's helper and measures
//! the pitch that arrives in the audio stream. Usage: pitch <device> [rate]

use std::time::Duration;

use aae_core::device::DeviceStore;
use aae_core::{emulator, sdk::Sdk};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let name = args.next().expect("device name");
    let rate: u32 = args.next().map(|r| r.parse().expect("rate")).unwrap_or(0);
    let sdk = Sdk::locate()?;
    let device = DeviceStore::open_default()?.get(&name)?;
    let (_, controller, adb) = emulator::attach(&sdk, &device).await?;
    let reader = {
        let controller = controller.clone();
        tokio::spawn(async move {
            let mut stream = controller.stream_audio(48_000, true).await.expect("stream");
            let mut left = Vec::new();
            while let Ok(Some(Ok(packet))) =
                tokio::time::timeout(Duration::from_secs(3), stream.next()).await
            {
                for frame in packet.audio.chunks_exact(4) {
                    left.push(i16::from_le_bytes([frame[0], frame[1]]));
                }
                if left.len() > 48_000 * 4 {
                    break;
                }
            }
            left
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = adb
        .shell(&format!(
            "am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver -a io.github.aaron_gh.aae.helper.PLAY_TONE --ei hz 1000 --ei rate {rate}"
        ))
        .await?;
    println!("{}", out.lines().last().unwrap_or(""));
    let left = reader.await?;
    // Count zero crossings over the loud part only.
    // The tone is the loud part: anything above a third of the loudest sample.
    let peak = left.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    let threshold = (peak / 3).max(50);
    println!("Loudest sample {peak} of 32768.");
    let loud: Vec<i16> = left
        .iter()
        .copied()
        .skip_while(|s| s.unsigned_abs() < threshold)
        .collect();
    let end = loud
        .iter()
        .rposition(|s| s.unsigned_abs() > threshold)
        .unwrap_or(0);
    let tone = &loud[..end];
    let crossings = tone.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let seconds = tone.len() as f64 / 48_000.0;
    if seconds < 0.2 {
        println!("No tone was heard in the stream.");
    } else {
        println!(
            "Measured {:.1} Hz over {:.2} seconds of tone (sent 1000 Hz).",
            crossings as f64 / 2.0 / seconds,
            seconds
        );
    }
    Ok(())
}
