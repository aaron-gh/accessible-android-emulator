//! Microphone input for a device: the host microphone, an audio file, or a
//! phone running AAE Remote, sent through gRPC injectAudio as 16-bit mono
//! real-time samples. The emulator needs an audio input backend for this to
//! work (see `emulator.rs`); without one the device records silence.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use crate::adb::Adb;
use crate::control::Controller;
use crate::error::{Error, Result};
use crate::proto::android::emulation::control as pb;

/// Sound going somewhere, and the task sending it on.
type Feed = (mpsc::Sender<Vec<i16>>, tokio::task::JoinHandle<Result<()>>);

/// How long after Android starts recording the emulator is ready for sound.
/// Sound sent sooner crashes it, as Android 16's emulator showed, where 400
/// ms was enough.
const SETTLE: Duration = Duration::from_millis(500);

/// The most the emulator takes.
const MAX_RATE: u32 = 48_000;

const RECORDING_WATCH: &str = "CLASSPATH=$(pm path io.github.aaron_gh.aae.helper | cut -d: -f2) \
     app_process / io.github.aaron_gh.aae.helper.ShellTool recording-watch";

/// Feeds mono 16-bit samples at `rate` into the device's microphone until the
/// sender is dropped. Returns the sender, and the task doing it, which ends
/// with an error if it can't go on.
///
/// The emulator takes sound only while Android is recording: sent at other
/// times, it's lost, or the emulator crashes. So AAE's helper says when the
/// device starts and stops recording, and sound goes in only while it is;
/// otherwise it's dropped. Needs helper 0.21.0 or later.
pub fn feed(controller: &Controller, adb: &Adb, rate: u32) -> Feed {
    let (tx, mut rx) = mpsc::channel::<Vec<i16>>(64);
    let (controller, adb) = (controller.clone(), adb.clone());
    let task = tokio::spawn(async move {
        let mut watch = RecordingWatch::start(&adb)?;
        let mut injecting: Option<Feed> = None;
        loop {
            tokio::select! {
                samples = rx.recv() => match samples {
                    Some(samples) => {
                        if let Some((sound, _)) = &injecting {
                            // Sound that can't keep up is dropped, not queued.
                            let _ = sound.try_send(samples);
                        }
                    }
                    None => return Ok(()),
                },
                recording = watch.next() => {
                    if recording? {
                        if injecting.is_none() {
                            injecting = Some(inject(&controller, rate, true));
                        }
                    } else {
                        // Ending the stream ends the injection.
                        injecting = None;
                    }
                }
            }
            if injecting
                .as_ref()
                .is_some_and(|(_, task)| task.is_finished())
                && let Some((_, task)) = injecting.take()
                && let Ok(Err(e)) = task.await
            {
                return Err(e);
            }
        }
    });
    (tx, task)
}

/// Says when the device starts and stops recording, through AAE's helper.
struct RecordingWatch {
    _child: tokio::process::Child,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
}

impl RecordingWatch {
    fn start(adb: &Adb) -> Result<RecordingWatch> {
        use tokio::io::AsyncBufReadExt;
        let mut child = tokio::process::Command::new(&adb.bin)
            .args(["-s", &adb.serial, "shell", RECORDING_WATCH])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Adb(format!("couldn't watch the device's microphone: {e}")))?;
        let stdout = child.stdout.take().expect("piped");
        Ok(RecordingWatch {
            _child: child,
            lines: tokio::io::BufReader::new(stdout).lines(),
        })
    }

    /// The next change: true when the device starts recording, false when it
    /// stops. The first says how it is now.
    async fn next(&mut self) -> Result<bool> {
        loop {
            match self.lines.next_line().await {
                Ok(Some(line)) => match line.trim() {
                    "recording" => return Ok(true),
                    "quiet" => return Ok(false),
                    _ => {}
                },
                _ => {
                    return Err(Error::Adb(
                        "AAE's helper couldn't watch the device's microphone".into(),
                    ));
                }
            }
        }
    }
}

/// One stream of sound into the emulator's microphone, from what's sent until
/// the sender is dropped. With `settle`, it waits [`SETTLE`] first, dropping
/// what's sent meanwhile; without, the caller has waited.
fn inject(controller: &Controller, rate: u32, settle: bool) -> Feed {
    use pb::audio_format::{Channels, DeliveryMode, SampleFormat};
    use tokio_stream::StreamExt;
    let (tx, mut rx) = mpsc::channel::<Vec<i16>>(64);
    let format = pb::AudioFormat {
        sampling_rate: rate as u64,
        channels: Channels::Mono as i32,
        format: SampleFormat::AudFmtS16 as i32,
        // Late samples are dropped, not queued.
        mode: DeliveryMode::ModeRealTime as i32,
    };
    let controller = controller.clone();
    let task = tokio::spawn(async move {
        if settle {
            tokio::time::sleep(SETTLE).await;
            // Recording may have stopped meanwhile. Drop samples queued
            // during the wait.
            if rx.is_closed() {
                return Ok(());
            }
            while rx.try_recv().is_ok() {}
        }
        // Keep the emulator's own host microphone access off.
        if let Err(e) = controller.allow_microphone(false).await {
            tracing::warn!("couldn't keep the emulator off the microphone: {e}");
        }
        let packets =
            tokio_stream::wrappers::ReceiverStream::new(rx).map(move |samples| pb::AudioPacket {
                format: Some(format),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_micros() as u64),
                audio: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
            });
        controller.inject_audio(packets).await
    });
    (tx, task)
}

/// A sound file, as mono 16-bit samples, for playing into a microphone.
pub struct Sound {
    pub samples: Vec<i16>,
    pub rate: u32,
}

impl Sound {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / self.rate as f64
    }
}

/// Reads a sound file: WAV, MP3, FLAC, Ogg Vorbis or M4A (AAC). Channels are
/// mixed to one, and sound above 48 kHz brought down to it.
pub fn decode(path: &std::path::Path) -> Result<Sound> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as Decode;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let unreadable = |e: &dyn std::fmt::Display| {
        Error::Audio(format!("{} can't be read as sound: {e}", path.display()))
    };
    let file = std::fs::File::open(path).map_err(|e| unreadable(&e))?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| {
            Error::Audio(format!(
                "{} isn't a sound file AAE can read. It reads WAV, MP3, FLAC, Ogg Vorbis and M4A.",
                path.display()
            ))
        })?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| unreadable(&"it has no sound"))?;
    let track_id = track.id;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| unreadable(&e))?;
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Decode::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Decode::ResetRequired) => break,
            Err(e) => return Err(unreadable(&e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                rate = spec.rate;
                let channels = spec.channels.count().max(1);
                let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
                buffer.copy_interleaved_ref(decoded);
                samples.extend(buffer.samples().chunks(channels).map(|frame| {
                    (frame.iter().map(|s| *s as i32).sum::<i32>() / frame.len() as i32) as i16
                }));
            }
            // A damaged packet is skipped.
            Err(Decode::DecodeError(_)) => continue,
            Err(e) => return Err(unreadable(&e)),
        }
    }
    if samples.is_empty() || rate == 0 {
        return Err(unreadable(&"it has no sound"));
    }
    if rate > MAX_RATE {
        samples = resample(&samples, rate, MAX_RATE);
        rate = MAX_RATE;
    }
    Ok(Sound { samples, rate })
}

/// Changes the sample rate, by straight lines between samples: plenty for
/// speech going into a phone's microphone.
fn resample(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    let length = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..length)
        .map(|i| {
            let at = i as f64 * step;
            let (index, fraction) = (at as usize, at.fract());
            let a = samples[index.min(samples.len() - 1)] as f64;
            let b = samples[(index + 1).min(samples.len() - 1)] as f64;
            (a + (b - a) * fraction) as i16
        })
        .collect()
}

/// How playing a sound into the microphone went.
pub enum Played {
    /// All of it went in.
    Whole,
    /// The app stopped listening this many seconds in.
    Stopped(f64),
}

/// Plays `sound` into the device's microphone from the start of the next
/// recording, or immediately if one is active. Returns when it has finished
/// or recording stops.
pub async fn play(controller: &Controller, adb: &Adb, sound: &Sound) -> Result<Played> {
    let mut watch = RecordingWatch::start(adb)?;
    while !watch.next().await? {}
    // The emulator needs a moment after recording starts (see SETTLE). An app
    // that stops listening meanwhile doesn't get the sound.
    tokio::select! {
        _ = tokio::time::sleep(SETTLE) => {}
        recording = watch.next() => {
            recording?;
            return Ok(Played::Stopped(0.0));
        }
    }
    let (tx, task) = inject(controller, sound.rate, false);
    let piece = (sound.rate / 50) as usize;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for (index, samples) in sound.samples.chunks(piece).enumerate() {
        tokio::select! {
            _ = tick.tick() => {}
            recording = watch.next() => {
                if !recording? {
                    drop(tx);
                    let _ = task.await;
                    return Ok(Played::Stopped(index as f64 / 50.0));
                }
            }
        }
        if tx.send(samples.to_vec()).await.is_err() {
            break;
        }
    }
    drop(tx);
    match task.await {
        Ok(Err(e)) => Err(Error::Audio(format!(
            "the emulator wouldn't take the sound: {e}"
        ))),
        _ => Ok(Played::Whole),
    }
}

/// The computer's microphone, playing into a device's.
pub struct Microphone {
    stop: Arc<AtomicBool>,
    /// The name of the microphone used, such as "MacBook Air Microphone".
    pub name: String,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl Microphone {
    /// Starts sending the computer's default microphone into the device,
    /// whenever an app on it records.
    pub async fn start(controller: &Controller, adb: &Adb) -> Result<Microphone> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(u32, String)>>();
        let (samples_tx, mut samples_rx) = mpsc::channel::<Vec<i16>>(64);
        {
            let stop = stop.clone();
            // cpal streams stay on the thread that made them.
            std::thread::Builder::new()
                .name("aae-microphone".into())
                .spawn(move || run_input(stop, samples_tx, ready_tx))
                .map_err(|e| Error::Audio(e.to_string()))?;
        }
        let (rate, name) = ready_rx
            .recv()
            .map_err(|_| Error::Audio("the microphone thread stopped".into()))??;
        let (feed, task) = feed(controller, adb, rate);
        // Passes captured sound on to the device until stopped.
        tokio::spawn(async move {
            while let Some(samples) = samples_rx.recv().await {
                if feed.send(samples).await.is_err() {
                    break;
                }
            }
        });
        Ok(Microphone { stop, name, task })
    }

    pub fn stop(self) {}
}

/// Stopping is dropping, so a microphone left on stops with its session.
impl Drop for Microphone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.task.abort();
    }
}

fn run_input(
    stop: Arc<AtomicBool>,
    samples: mpsc::Sender<Vec<i16>>,
    ready: std::sync::mpsc::Sender<Result<(u32, String)>>,
) {
    let opened = (|| -> Result<(cpal::Stream, u32, String)> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| {
            Error::Audio("this computer has no microphone, or AAE isn't allowed to use it".into())
        })?;
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "the microphone".into());
        // At most 48 kHz, which the emulator takes.
        let supported = device
            .supported_input_configs()
            .map_err(|e| Error::Audio(e.to_string()))?
            .filter(|c| c.min_sample_rate() <= MAX_RATE)
            .max_by_key(|c| c.max_sample_rate().min(MAX_RATE))
            .ok_or_else(|| {
                Error::Audio("the microphone doesn't record at 48 kHz or less".into())
            })?;
        let rate = supported.max_sample_rate().min(MAX_RATE);
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.with_sample_rate(rate).config();
        let channels = config.channels as usize;
        let error = |e| tracing::warn!("microphone: {e}");
        macro_rules! stream {
            ($t:ty, $to_i16:expr) => {{
                let samples = samples.clone();
                device.build_input_stream(
                    config.clone(),
                    move |data: &[$t], _: &cpal::InputCallbackInfo| {
                        // Mono: the channels averaged.
                        let mono: Vec<i16> = data
                            .chunks(channels.max(1))
                            .map(|frame| {
                                let sum: i32 = frame.iter().map(|s| $to_i16(*s) as i32).sum();
                                (sum / frame.len().max(1) as i32) as i16
                            })
                            .collect();
                        let _ = samples.try_send(mono);
                    },
                    error,
                    None,
                )
            }};
        }
        let stream = match format {
            cpal::SampleFormat::F32 => stream!(f32, |s: f32| (s.clamp(-1.0, 1.0) * 32767.0) as i16),
            cpal::SampleFormat::I16 => stream!(i16, |s: i16| s),
            cpal::SampleFormat::I32 => stream!(i32, |s: i32| (s >> 16) as i16),
            cpal::SampleFormat::U16 => stream!(u16, |s: u16| (s as i32 - 32768) as i16),
            other => {
                return Err(Error::Audio(format!(
                    "the microphone uses an unsupported sample format, {other}"
                )));
            }
        }
        .map_err(|e| Error::Audio(e.to_string()))?;
        stream.play().map_err(|e| Error::Audio(e.to_string()))?;
        Ok((stream, rate, name))
    })();
    match opened {
        Ok((stream, rate, name)) => {
            let _ = ready.send(Ok((rate, name)));
            while !stop.load(Ordering::Relaxed) && !samples.is_closed() {
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(stream);
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    }
}

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const MIC_LEVEL: &str = "io.github.aaron_gh.aae.helper.MIC_LEVEL";

/// Peak microphone level recorded by the helper over `ms` milliseconds,
/// 0 to 32767. Nothing is kept.
pub async fn level(adb: &crate::adb::Adb, ms: u32) -> Result<u32> {
    // A runtime permission, which the shell may grant.
    let _ = adb
        .shell("pm grant io.github.aaron_gh.aae.helper android.permission.RECORD_AUDIO")
        .await;
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {MIC_LEVEL} --ei ms {ms}"
        ))
        .await?;
    let data = out
        .split_once("data=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(data, _)| data)
        .unwrap_or("");
    if !out.contains("result=1") {
        return Err(Error::Adb(format!(
            "AAE's helper couldn't record from the device's microphone: {}",
            out.trim()
        )));
    }
    data.trim()
        .parse()
        .map_err(|_| Error::Adb(format!("AAE's helper said {data}")))
}

/// Minimum peak for [`check`] to pass. The tone is about 16000; room noise
/// is a few hundred.
pub const HEARD: u32 = 2000;

/// What [`check`] found, to say.
pub fn describe_check(heard: u32) -> Result<String> {
    if heard > HEARD {
        Ok(format!(
            "The device recorded the test tone (peak {heard} of 32767): microphone input works."
        ))
    } else {
        Err(Error::Audio(format!(
            "The device didn't record the test tone (peak {heard} of 32767): microphone input isn't reaching it."
        )))
    }
}

/// Injects a tone and records it on the device. Returns the recorded peak.
pub async fn check(controller: &Controller, adb: &crate::adb::Adb) -> Result<u32> {
    let rate = 48_000u32;
    let (tx, task) = feed(controller, adb, rate);
    // 440 Hz at half amplitude, 20 ms frames in real time, for 2.5 s: longer
    // than the recording.
    let tone = tokio::spawn(async move {
        let piece = (rate / 50) as usize;
        let mut phase = 0f32;
        let step = 2.0 * std::f32::consts::PI * 440.0 / rate as f32;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        for _ in 0..125 {
            tick.tick().await;
            let samples: Vec<i16> = (0..piece)
                .map(|_| {
                    phase += step;
                    (phase.sin() * 16_000.0) as i16
                })
                .collect();
            if tx.send(samples).await.is_err() {
                break;
            }
        }
    });
    // Start recording after the tone; injection starts when recording does.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let heard = level(adb, 1500).await;
    tone.abort();
    // The emulator may have refused the sound: say why.
    if task.is_finished() {
        if let Ok(Err(e)) = task.await {
            return Err(Error::Audio(format!(
                "the emulator wouldn't take the sound: {e}"
            )));
        }
    } else {
        task.abort();
    }
    heard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_length_in_time_and_the_straight_lines() {
        let ramp: Vec<i16> = (0..96).map(|i| i * 100).collect();
        let half = resample(&ramp, 96_000, 48_000);
        assert_eq!(half.len(), 48);
        assert_eq!(half[10], 2000);
    }

    #[test]
    fn a_file_that_isnt_sound_says_so() {
        let path = std::env::temp_dir().join("aae-not-sound.txt");
        std::fs::write(&path, "not sound").unwrap();
        let error = decode(&path).err().unwrap().to_string();
        assert!(error.contains("isn't a sound file"), "{error}");
    }
}
