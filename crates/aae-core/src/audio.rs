//! Playing a device's audio on the host.
//!
//! AAE does not use the emulator's own audio output, which is behind most of
//! the silent, crackling or lagging audio people get on macOS. It takes the raw
//! audio stream from the emulator's gRPC interface and plays it with cpal, the
//! host's native audio API, through a small jitter buffer. The buffer is kept
//! short so speech stays close to the key press that caused it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use tokio_stream::StreamExt;

use crate::control::Controller;
use crate::error::{Error, Result};

/// The highest rate the emulator produces. Android itself rarely goes above 48 kHz.
const MAX_SOURCE_RATE: u32 = 48_000;
/// Audio buffered before playback starts, and after a gap. Smooths network jitter.
const PREFILL: Duration = Duration::from_millis(30);
/// If more than this is buffered, the oldest audio is dropped to catch up.
const MAX_BUFFERED: Duration = Duration::from_millis(120);

/// Counters for the audio health check.
#[derive(Debug, Default)]
pub struct AudioStats {
    pub packets: AtomicU64,
    /// Times playback ran out of audio mid-sound.
    pub underruns: AtomicU64,
    /// Samples dropped to keep latency low.
    pub dropped: AtomicU64,
    started: OnceInstant,
    last_packet_ms: AtomicU64,
    /// Sum and count of delays between the device capturing a packet and AAE
    /// receiving it, in microseconds.
    delay_sum_us: AtomicU64,
    delay_count: AtomicU64,
    /// The loudest sample seen, from 0 to 1000.
    peak: AtomicU32,
}

impl AudioStats {
    /// Time since the last packet arrived from the device, if any has.
    pub fn since_last_packet(&self) -> Option<Duration> {
        let last = self.last_packet_ms.load(Ordering::Relaxed);
        (last > 0).then(|| {
            self.started
                .elapsed()
                .saturating_sub(Duration::from_millis(last))
        })
    }

    /// The average delay from the device capturing audio to AAE receiving it.
    /// Playback adds the jitter buffer and the host's output latency on top.
    pub fn average_delay(&self) -> Option<Duration> {
        let count = self.delay_count.load(Ordering::Relaxed);
        (count > 0)
            .then(|| Duration::from_micros(self.delay_sum_us.load(Ordering::Relaxed) / count))
    }

    /// The loudest sample received so far, from 0.0 to 1.0.
    pub fn peak(&self) -> f32 {
        self.peak.load(Ordering::Relaxed) as f32 / 1000.0
    }

    fn mark_packet(&self, captured_us: u64, audio: &[u8]) {
        self.peak
            .fetch_max((level(audio) * 1000.0) as u32, Ordering::Relaxed);
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        // Ignore missing or nonsense timestamps.
        if captured_us > 0 && now_us >= captured_us && now_us - captured_us < 10_000_000 {
            self.delay_sum_us
                .fetch_add(now_us - captured_us, Ordering::Relaxed);
            self.delay_count.fetch_add(1, Ordering::Relaxed);
        }
        self.packets.fetch_add(1, Ordering::Relaxed);
        let ms = self.started.elapsed().as_millis().max(1) as u64;
        self.last_packet_ms.store(ms, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct OnceInstant(Instant);

impl Default for OnceInstant {
    fn default() -> Self {
        OnceInstant(Instant::now())
    }
}

impl OnceInstant {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Shared playback controls.
#[derive(Debug)]
struct Controls {
    /// Volume from 0.0 to 1.0, stored as f32 bits.
    volume: AtomicU32,
    muted: AtomicBool,
    stop: AtomicBool,
}

/// Plays one device's audio until dropped or stopped.
pub struct AudioPlayer {
    controls: Arc<Controls>,
    pub stats: Arc<AudioStats>,
    /// The host output's sample rate and channel count.
    pub output_rate: u32,
    pub output_channels: u16,
    receiver: tokio::task::JoinHandle<()>,
    output: Option<std::thread::JoinHandle<()>>,
}

impl AudioPlayer {
    /// Starts playing the device's audio on the host's default output.
    pub async fn start(controller: &Controller) -> Result<Self> {
        let controls = Arc::new(Controls {
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let stats = Arc::new(AudioStats::default());

        // cpal streams can't move between threads on every platform, so the
        // output lives on its own thread for its whole life.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let output = {
            let controls = controls.clone();
            let stats = stats.clone();
            std::thread::Builder::new()
                .name("aae-audio-out".into())
                .spawn(move || run_output(controls, stats, ready_tx))
                .map_err(|e| Error::Audio(e.to_string()))?
        };
        tracing::debug!("waiting for the audio output to open");
        let (mut producer, output_rate, output_channels) = ready_rx
            .recv()
            .map_err(|_| Error::Audio("the audio output thread stopped".into()))??;

        let source_rate = output_rate.min(MAX_SOURCE_RATE);
        tracing::debug!(
            output_rate,
            output_channels,
            source_rate,
            "audio output open"
        );
        let receiver = {
            let stats = stats.clone();
            let controls = controls.clone();
            let controller = controller.clone();
            tokio::spawn(async move {
                // The emulator answers the request only once the device makes its
                // first sound, so this waits here rather than in start().
                let mut stream = match controller.stream_audio(source_rate, true).await {
                    Ok(stream) => stream,
                    Err(e) => {
                        tracing::warn!("could not start the device's audio stream: {e}");
                        return;
                    }
                };
                tracing::debug!("device audio stream started");
                let mut resampler = Resampler::new(source_rate, output_rate);
                let mut samples = Vec::new();
                while let Some(Ok(packet)) = stream.next().await {
                    if controls.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    stats.mark_packet(packet.timestamp, &packet.audio);
                    samples.clear();
                    for pair in packet.audio.chunks_exact(4) {
                        let left = i16::from_le_bytes([pair[0], pair[1]]);
                        let right = i16::from_le_bytes([pair[2], pair[3]]);
                        resampler.push(left, right, &mut samples);
                    }
                    let pushed = producer.push_slice(&samples);
                    if pushed < samples.len() {
                        stats
                            .dropped
                            .fetch_add((samples.len() - pushed) as u64, Ordering::Relaxed);
                    }
                }
            })
        };

        Ok(AudioPlayer {
            controls,
            stats,
            output_rate,
            output_channels,
            receiver,
            output: Some(output),
        })
    }

    pub fn set_volume(&self, volume: f32) {
        self.controls
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.controls.volume.load(Ordering::Relaxed))
    }

    /// Mutes or unmutes. Returns the new state.
    pub fn toggle_mute(&self) -> bool {
        !self.controls.muted.fetch_xor(true, Ordering::Relaxed)
    }

    pub fn is_muted(&self) -> bool {
        self.controls.muted.load(Ordering::Relaxed)
    }

    /// True while the receiving task is alive.
    pub fn is_running(&self) -> bool {
        !self.receiver.is_finished()
    }

    pub fn stop(&mut self) {
        self.controls.stop.store(true, Ordering::Relaxed);
        self.receiver.abort();
        if let Some(thread) = self.output.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

type SampleProducer = <HeapRb<i16> as Split>::Prod;
type SampleConsumer = <HeapRb<i16> as Split>::Cons;
type Ready = std::result::Result<(SampleProducer, u32, u16), Error>;

/// Opens the default output and plays from the ring buffer until told to stop.
fn run_output(
    controls: Arc<Controls>,
    stats: Arc<AudioStats>,
    ready: std::sync::mpsc::Sender<Ready>,
) {
    let opened = (|| -> Result<(cpal::Stream, SampleProducer, u32, u16)> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(Error::NoAudioOutput)?;
        let supported = device
            .default_output_config()
            .map_err(|e| Error::Audio(e.to_string()))?;
        let config: cpal::StreamConfig = supported.config();
        let rate = config.sample_rate;
        let channels = config.channels;

        // Room for half a second of stereo audio.
        let ring = HeapRb::<i16>::new(rate as usize);
        let (producer, consumer) = ring.split();
        let player = Playback {
            consumer,
            controls: controls.clone(),
            stats: stats.clone(),
            channels: channels as usize,
            prefill: frames(rate, PREFILL) * 2,
            max_buffered: frames(rate, MAX_BUFFERED) * 2,
            playing: false,
        };
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, player),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, player),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, player),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, player),
            other => {
                return Err(Error::Audio(format!(
                    "the output uses an unsupported sample format, {other}"
                )));
            }
        }?;
        stream.play().map_err(|e| Error::Audio(e.to_string()))?;
        Ok((stream, producer, rate, channels))
    })();

    match opened {
        Ok((stream, producer, rate, channels)) => {
            let _ = ready.send(Ok((producer, rate, channels)));
            while !controls.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(stream);
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    }
}

fn frames(rate: u32, duration: Duration) -> usize {
    (rate as u64 * duration.as_millis() as u64 / 1000) as usize
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut player: Playback,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| player.fill(out),
            |e| tracing::warn!("audio output error: {e}"),
            None,
        )
        .map_err(|e| Error::Audio(e.to_string()))
}

/// The output callback's state.
struct Playback {
    consumer: SampleConsumer,
    controls: Arc<Controls>,
    stats: Arc<AudioStats>,
    channels: usize,
    /// Stereo samples (not frames) to buffer before starting.
    prefill: usize,
    max_buffered: usize,
    playing: bool,
}

impl Playback {
    fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T]) {
        let buffered = self.consumer.occupied_len();
        if buffered > self.max_buffered {
            // Fell behind: drop the oldest audio, keeping whole frames.
            let excess = (buffered - self.prefill) & !1;
            self.consumer.skip(excess);
            self.stats
                .dropped
                .fetch_add(excess as u64, Ordering::Relaxed);
        }
        if !self.playing && self.consumer.occupied_len() >= self.prefill {
            self.playing = true;
        }
        let gain = if self.controls.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.controls.volume.load(Ordering::Relaxed))
        };
        let silence = T::from_sample(0.0f32);
        for frame in out.chunks_mut(self.channels) {
            let mut lr = [0i16; 2];
            if self.playing && self.consumer.pop_slice(&mut lr) < 2 {
                // Ran dry. Wait for the buffer to refill before resuming.
                self.playing = false;
                self.stats.underruns.fetch_add(1, Ordering::Relaxed);
                lr = [0, 0];
            }
            let left = lr[0] as f32 / 32768.0 * gain;
            let right = lr[1] as f32 / 32768.0 * gain;
            match frame.len() {
                1 => frame[0] = T::from_sample((left + right) / 2.0),
                _ => {
                    frame[0] = T::from_sample(left);
                    frame[1] = T::from_sample(right);
                    for extra in &mut frame[2..] {
                        *extra = silence;
                    }
                }
            }
        }
    }
}

/// Linear-interpolation resampler for interleaved stereo, used only when the
/// host output runs faster than the emulator can produce.
struct Resampler {
    step: f64,
    position: f64,
    previous: (i16, i16),
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Resampler {
            step: from as f64 / to as f64,
            position: 0.0,
            previous: (0, 0),
        }
    }

    fn push(&mut self, left: i16, right: i16, out: &mut Vec<i16>) {
        if self.step == 1.0 {
            out.push(left);
            out.push(right);
            return;
        }
        // Emit every output sample that falls between the previous input and this one.
        while self.position < 1.0 {
            let t = self.position as f32;
            out.push(lerp(self.previous.0, left, t));
            out.push(lerp(self.previous.1, right, t));
            self.position += self.step;
        }
        self.position -= 1.0;
        self.previous = (left, right);
    }
}

fn lerp(a: i16, b: i16, t: f32) -> i16 {
    (a as f32 + (b as f32 - a as f32) * t) as i16
}

/// Measures the time from pressing a key to hearing the device respond.
///
/// For each trial it waits for quiet, presses the next key in `keys` (cycling
/// through them), and times how long it is until a loud packet arrives.
/// Returns one result per trial; `None` means nothing was heard within `timeout`.
pub async fn measure_key_to_sound(
    controller: &Controller,
    keys: &[crate::keys::Key],
    trials: usize,
    timeout: Duration,
) -> Result<Vec<Option<Duration>>> {
    const THRESHOLD: f32 = 0.02;
    const QUIET: Duration = Duration::from_millis(500);

    // Packets arrive only while the device makes sound, and the request itself
    // completes only with the first one, so read them on a task of their own.
    let (tx, mut packets) = tokio::sync::mpsc::unbounded_channel();
    let reader = {
        let controller = controller.clone();
        tokio::spawn(async move {
            let Ok(mut stream) = controller.stream_audio(MAX_SOURCE_RATE, true).await else {
                return;
            };
            while let Some(Ok(packet)) = stream.next().await {
                if tx.send(level(&packet.audio)).is_err() {
                    break;
                }
            }
        })
    };

    let mut results = Vec::with_capacity(trials);
    for key in keys.iter().cycle().take(trials) {
        // Wait until nothing loud has arrived for a while, silence included.
        loop {
            match tokio::time::timeout(QUIET, packets.recv()).await {
                Err(_) => break,
                Ok(None) => return Err(Error::Audio("the audio stream ended".into())),
                Ok(Some(_)) => {}
            }
        }
        while packets.try_recv().is_ok() {}
        let pressed = Instant::now();
        controller.press(key).await?;
        let mut heard = None;
        while let Some(remaining) = timeout.checked_sub(pressed.elapsed()) {
            match tokio::time::timeout(remaining, packets.recv()).await {
                Ok(Some(peak)) if peak > THRESHOLD => {
                    heard = Some(pressed.elapsed());
                    break;
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        results.push(heard);
    }
    reader.abort();
    Ok(results)
}

/// The peak level of 16-bit little-endian samples, from 0.0 to 1.0.
fn level(audio: &[u8]) -> f32 {
    audio
        .chunks_exact(2)
        .map(|s| (i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).abs())
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_passes_through_at_equal_rates() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.push(1, 2, &mut out);
        assert_eq!(out, vec![1, 2]);
    }

    #[test]
    fn resampler_doubles_samples_when_upsampling_twofold() {
        let mut r = Resampler::new(48_000, 96_000);
        let mut out = Vec::new();
        for i in 0..100 {
            r.push(i, i, &mut out);
        }
        assert_eq!(out.len(), 400);
    }
}
