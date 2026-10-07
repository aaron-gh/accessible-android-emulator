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
/// Audio buffered before playback starts, and after a gap. Smooths jitter:
/// the emulator sends about 10 ms of audio every 11 ms, rarely more than
/// 23 ms apart, so twice a packet is enough.
const PREFILL: Duration = Duration::from_millis(20);
/// If more than this is buffered, the oldest audio is dropped to catch up,
/// so delay can't build up if the emulator runs slightly fast.
const MAX_BUFFERED: Duration = Duration::from_millis(60);
/// The Mac's output buffer, asked for in frames: about 5 ms at 48 kHz. The
/// system default is about twice that.
const OUTPUT_FRAMES: u32 = 256;

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
    /// The loudest sample since the health check last reset it, 0 to 1000.
    recent_peak: AtomicU32,
    /// Errors from the Mac's audio output, such as the output going away.
    pub output_errors: AtomicU64,
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
        let level = (level(audio) * 1000.0) as u32;
        self.peak.fetch_max(level, Ordering::Relaxed);
        self.recent_peak.fetch_max(level, Ordering::Relaxed);
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
    /// Silenced because another device is the one in use. Separate from
    /// `muted`, which the user sets.
    background: AtomicBool,
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
    /// Starts playing the device's audio on the host's default output.
    ///
    /// `speed` is how fast the device really plays its audio, as
    /// [`measure_speed`] finds: 1.0 when it's right. Some older Android images
    /// play about 8% slow in the emulator, which lowers the pitch; for those,
    /// AAE raises the pitch back by the same amount, keeping the timing.
    pub async fn start_with_speed(controller: &Controller, speed: f64) -> Result<Self> {
        Self::start_inner(controller, speed).await
    }

    pub async fn start(controller: &Controller) -> Result<Self> {
        Self::start_inner(controller, 1.0).await
    }

    async fn start_inner(controller: &Controller, speed: f64) -> Result<Self> {
        let controls = Arc::new(Controls {
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            background: AtomicBool::new(false),
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
                let mut pitch = PitchCorrection::new(speed, output_rate);
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
                    if let Some(pitch) = pitch.as_mut() {
                        pitch.process(&mut samples);
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

    /// Silences this device while another is the one in use (true), or
    /// plays it again (false). Doesn't touch the user's own mute.
    pub fn set_background(&self, background: bool) {
        self.controls
            .background
            .store(background, Ordering::Relaxed);
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
        let mut config: cpal::StreamConfig = supported.config();
        // A smaller output buffer, when the output allows it.
        if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
            config.buffer_size = cpal::BufferSize::Fixed(OUTPUT_FRAMES.clamp(*min, *max));
        }
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
    let stats = player.stats.clone();
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| player.fill(out),
            move |e| {
                stats.output_errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!("audio output error: {e}");
            },
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
        let gain = if self.controls.muted.load(Ordering::Relaxed)
            || self.controls.background.load(Ordering::Relaxed)
        {
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

/// Raises the pitch of audio from a device that plays too slowly, keeping its
/// timing, with Signalsmith Stretch.
struct PitchCorrection {
    stretch: signalsmith_stretch::Stretch,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl PitchCorrection {
    /// None when the speed is close enough to right that correcting it would
    /// do more harm than good.
    fn new(speed: f64, rate: u32) -> Option<Self> {
        if !(0.5..2.0).contains(&speed) || (speed - 1.0).abs() < 0.01 {
            return None;
        }
        // The default preset's larger blocks sound cleaner than the cheaper
        // one, for a little more delay, and only old Android needs this.
        let mut stretch = signalsmith_stretch::Stretch::preset_default(2, rate);
        stretch.set_transpose_factor((1.0 / speed) as f32, None);
        Some(PitchCorrection {
            stretch,
            input: Vec::new(),
            output: Vec::new(),
        })
    }

    /// Corrects interleaved stereo samples in place.
    fn process(&mut self, samples: &mut [i16]) {
        self.input.clear();
        self.input
            .extend(samples.iter().map(|&s| s as f32 / 32768.0));
        self.output.resize(samples.len(), 0.0);
        self.stretch.process(&self.input, &mut self.output);
        for (out, &value) in samples.iter_mut().zip(&self.output) {
            *out = (value * 32768.0).clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// The speed a known emulator fault plays at: images that mix at 48 kHz
/// through the old goldfish audio device, which runs at 44.1 kHz.
const GOLDFISH_SPEED: f64 = 44_100.0 / 48_000.0;

/// Measures how fast the device really plays audio: 1.0 when it's right.
///
/// AAE's helper plays a 1,000 Hz tone inside the device while AAE listens to
/// the audio stream and finds the frequency that actually arrives. Nobody
/// hears it: call this only while AAE isn't playing the device's audio, and
/// the emulator's own output is off. A result close to a known fault snaps to
/// its exact value.
/// The name of the Mac's sound output, or None if it has none.
/// The device's audio as 16-bit stereo samples at `rate`, interleaved, for
/// sending elsewhere instead of playing here, as to AAE's Android app.
/// Older Android's slow audio is corrected as for playing (see
/// [`AudioPlayer::start_with_speed`]). Ends when the receiver is dropped, or
/// the device stops.
pub async fn pcm_stream(
    controller: &Controller,
    rate: u32,
    speed: f64,
) -> Result<tokio::sync::mpsc::Receiver<Vec<i16>>> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let controller = controller.clone();
    tokio::spawn(async move {
        // As for playing: the emulator answers once the device first sounds.
        let mut stream = match controller.stream_audio(rate, true).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!("could not start the device's audio stream: {e}");
                return;
            }
        };
        let mut pitch = PitchCorrection::new(speed, rate);
        while let Some(Ok(packet)) = stream.next().await {
            let mut samples: Vec<i16> = packet
                .audio
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            if let Some(pitch) = pitch.as_mut() {
                pitch.process(&mut samples);
            }
            if tx.send(samples).await.is_err() {
                break;
            }
        }
    });
    Ok(rx)
}

pub fn output_name() -> Option<String> {
    let device = cpal::default_host().default_output_device()?;
    Some(
        device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "the default output".into()),
    )
}

/// What the audio health check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioHealth {
    /// The device's sound is reaching AAE.
    Working,
    /// Nothing suggests a problem, but nothing proves the sound arrives:
    /// the device hasn't played anything lately.
    Quiet,
    /// The device's sound isn't reaching AAE, and why, in words.
    Broken(String),
}

/// How long ago Android last played sound on any output, from
/// `dumpsys media.audio_flinger`, or None if it doesn't say.
pub async fn android_last_played(adb: &crate::adb::Adb) -> Option<Duration> {
    last_played(&adb.shell("dumpsys media.audio_flinger").await.ok()?)
}

fn last_played(dump: &str) -> Option<Duration> {
    dump.lines()
        .filter_map(|l| l.trim().strip_prefix("Last write occurred (msecs):"))
        .filter_map(|ms| ms.trim().parse::<u64>().ok())
        .min()
        .map(Duration::from_millis)
}

impl AudioPlayer {
    /// Checks without making a sound: AAE's connection is alive, the Mac's
    /// output has had no errors, and if Android played something in the last
    /// few seconds, it arrived.
    pub async fn check(&self, adb: &crate::adb::Adb) -> AudioHealth {
        if !self.is_running() {
            return AudioHealth::Broken(
                "AAE's connection to the device's audio has stopped.".into(),
            );
        }
        if self.stats.output_errors.load(Ordering::Relaxed) > 0 {
            return AudioHealth::Broken(
                "The Mac's audio output reported errors, perhaps because it changed.".into(),
            );
        }
        let window = Duration::from_secs(5);
        let Some(played) = android_last_played(adb).await else {
            return AudioHealth::Quiet;
        };
        // Only sound played since AAE started listening can have arrived.
        if played > window || self.stats.started.elapsed() < played + Duration::from_secs(3) {
            return AudioHealth::Quiet;
        }
        // Android played something just now; it should have arrived, allowing
        // a moment for it to travel.
        match self.stats.since_last_packet() {
            Some(gap) if gap <= played + Duration::from_secs(2) => AudioHealth::Working,
            _ => AudioHealth::Broken(
                "The device is playing sound, but none of it is reaching AAE.".into(),
            ),
        }
    }

    /// Checks by making a sound: AAE's helper plays its test tone, with
    /// AAE's own playback muted so nobody hears it, and the tone has to
    /// arrive within a few seconds.
    pub async fn probe(&self, adb: &crate::adb::Adb) -> AudioHealth {
        if let AudioHealth::Broken(why) = self.check(adb).await {
            return AudioHealth::Broken(why);
        }
        let was_muted = self.is_muted();
        if !was_muted {
            self.toggle_mute();
        }
        self.stats.recent_peak.store(0, Ordering::Relaxed);
        let played = adb
            .shell(
                "am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
                 -a io.github.aaron_gh.aae.helper.PLAY_TONE --ei hz 1000",
            )
            .await;
        let mut heard = false;
        if played
            .as_ref()
            .is_ok_and(|out| out.contains("result=") && !out.contains("result=0"))
        {
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if self.stats.recent_peak.load(Ordering::Relaxed) > 100 {
                    heard = true;
                    break;
                }
            }
            // The tone lasts a second and a half; stay muted until it's over.
            tokio::time::sleep(Duration::from_millis(1600)).await;
        }
        if !was_muted {
            self.toggle_mute();
        }
        match played {
            Err(_) => AudioHealth::Broken("AAE's helper couldn't play its test tone.".into()),
            Ok(out) if !out.contains("result=") || out.contains("result=0") => {
                AudioHealth::Broken("AAE's helper couldn't play its test tone.".into())
            }
            Ok(_) if heard => AudioHealth::Working,
            Ok(_) => AudioHealth::Broken(
                "The device played a test tone, but it didn't reach AAE.".into(),
            ),
        }
    }
}

pub async fn measure_speed(controller: &Controller, adb: &crate::adb::Adb) -> Result<f64> {
    const TONE_HZ: f64 = 1000.0;
    let reader = {
        let controller = controller.clone();
        tokio::spawn(async move {
            let mut left = Vec::new();
            // The stream only answers once the device makes a sound.
            let Ok(Ok(mut stream)) = tokio::time::timeout(
                Duration::from_secs(8),
                controller.stream_audio(MAX_SOURCE_RATE, true),
            )
            .await
            else {
                return left;
            };
            while let Ok(Some(Ok(packet))) =
                tokio::time::timeout(Duration::from_secs(3), stream.next()).await
            {
                for frame in packet.audio.chunks_exact(4) {
                    left.push(i16::from_le_bytes([frame[0], frame[1]]) as f32);
                }
                if left.len() > MAX_SOURCE_RATE as usize * 3 {
                    break;
                }
            }
            left
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = adb
        .shell(&format!(
            "am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
             -a io.github.aaron_gh.aae.helper.PLAY_TONE --ei hz {TONE_HZ}"
        ))
        .await?;
    if !out.contains("result=") || out.contains("result=0") {
        return Err(Error::Audio(
            "AAE's helper couldn't play its test tone.".into(),
        ));
    }
    let samples = reader.await.map_err(|e| Error::Audio(e.to_string()))?;
    if samples.len() < MAX_SOURCE_RATE as usize / 2 {
        return Err(Error::Audio(
            "The device's audio stream gave too little to measure.".into(),
        ));
    }
    // Find which speed between 85% and 115% puts the most energy at the
    // tone's frequency. Speech playing at the same time hardly moves it.
    let mut best = (1.0, 0.0);
    let mut speed = 0.85;
    while speed <= 1.15 {
        let energy = goertzel(&samples, TONE_HZ * speed, MAX_SOURCE_RATE as f64);
        if energy > best.1 {
            best = (speed, energy);
        }
        speed += 0.0025;
    }
    let measured = best.0;
    Ok(if (measured - GOLDFISH_SPEED).abs() < 0.015 {
        GOLDFISH_SPEED
    } else if (measured - 1.0).abs() < 0.015 {
        1.0
    } else {
        measured
    })
}

/// The energy of one frequency in a signal (the Goertzel algorithm).
fn goertzel(samples: &[f32], hz: f64, rate: f64) -> f64 {
    let coefficient = 2.0 * (2.0 * std::f64::consts::PI * hz / rate).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 + coefficient * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coefficient * s1 * s2
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
/// Presses keys in turn and times how long until the device makes a sound,
/// for each press. Before each press it waits for quiet, up to a few
/// seconds. Stops early, keeping the results so far, once `limit` has passed
/// or `stop` is set, so it can never run on.
pub async fn measure_key_to_sound(
    controller: &Controller,
    keys: &[crate::keys::Key],
    trials: usize,
    timeout: Duration,
    limit: Duration,
    stop: &AtomicBool,
) -> Result<Vec<Option<Duration>>> {
    const THRESHOLD: f32 = 0.02;
    const QUIET: Duration = Duration::from_millis(500);
    const MAX_WAIT_FOR_QUIET: Duration = Duration::from_secs(8);

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

    let started = Instant::now();
    let mut results = Vec::with_capacity(trials);
    for key in keys.iter().cycle().take(trials) {
        if stop.load(Ordering::Relaxed) || started.elapsed() >= limit {
            break;
        }
        // Wait for half a second without loud sound (silent packets count as
        // quiet), but not for ever: speech with long hints can go on.
        let waiting = Instant::now();
        let mut quiet_since = Instant::now();
        while quiet_since.elapsed() < QUIET && waiting.elapsed() < MAX_WAIT_FOR_QUIET {
            match tokio::time::timeout(QUIET, packets.recv()).await {
                Err(_) => break,
                Ok(None) => return Err(Error::Audio("the audio stream ended".into())),
                Ok(Some(peak)) if peak > THRESHOLD => quiet_since = Instant::now(),
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
    fn reads_when_android_last_played() {
        let dump = "Output thread 0x1, name AudioOut_D:\n  Standby: yes\n  Last write occurred (msecs): 568184\n\
                    Output thread 0x2, name AudioOut_15:\n  Last write occurred (msecs): 1200\n";
        assert_eq!(last_played(dump), Some(Duration::from_millis(1200)));
        assert_eq!(last_played("Output thread 0x3:\n  Total writes: 0\n"), None);
    }

    #[test]
    fn goertzel_finds_the_tone() {
        let rate = 48_000.0;
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f64::consts::PI * 918.75 * i as f64 / rate).sin() as f32)
            .collect();
        assert!(goertzel(&tone, 918.75, rate) > 100.0 * goertzel(&tone, 1000.0, rate));
    }

    #[test]
    fn pitch_correction_only_when_needed() {
        assert!(PitchCorrection::new(1.0, 48_000).is_none());
        assert!(PitchCorrection::new(1.005, 48_000).is_none());
        assert!(PitchCorrection::new(GOLDFISH_SPEED, 48_000).is_some());
    }

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
