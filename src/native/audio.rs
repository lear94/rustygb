//! Bridge from the core's mono samples to the default audio device.
//!
//! The emulation thread pushes into a bounded channel; the device callback pops without
//! blocking and plays silence when it runs dry.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, ErrorKind, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};
use rusty_gb::GameBoy;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Bridge capacity as a fraction of a second (125 ms), the latency bound.
const BRIDGE_FRACTION: u32 = 8;
/// Silence queued after running dry (~62 ms): several frames, because Windows timers wake
/// up to 16 ms late.
const PREFILL_FRACTION: u32 = 16;
const SCRATCH_LEN_SAMPLES: usize = 1024;

/// Output configuration this front-end cannot use.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub(super) enum AudioError {
    /// The host has no default output.
    NoDevice,
    /// The device wants samples in a format not converted here.
    UnsupportedFormat(SampleFormat),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no output device"),
            Self::UnsupportedFormat(format) => write!(f, "unsupported sample format {format}"),
        }
    }
}

impl Error for AudioError {}

pub(super) struct AudioOutput {
    /// Playback stops when the stream is dropped.
    _stream: Stream,
    sender: Sender<f32>,
    lost: Arc<AtomicBool>,
    sample_rate_hz: u32,
    prefill_samples: u32,
    scratch: [f32; SCRATCH_LEN_SAMPLES],
}

impl AudioOutput {
    pub(super) fn open() -> Result<Self, Box<dyn Error>> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let sample_rate_hz = config.sample_rate;
        let lost = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = bounded(usize::try_from(sample_rate_hz / BRIDGE_FRACTION)?);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, receiver, &lost),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, receiver, &lost),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, receiver, &lost),
            SampleFormat::I32 => build_stream::<i32>(&device, &config, receiver, &lost),
            SampleFormat::U8 => build_stream::<u8>(&device, &config, receiver, &lost),
            other => return Err(AudioError::UnsupportedFormat(other).into()),
        }?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            sender,
            lost,
            sample_rate_hz,
            prefill_samples: sample_rate_hz / PREFILL_FRACTION,
            scratch: [0.0; SCRATCH_LEN_SAMPLES],
        })
    }

    pub(super) fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Moves every pending core sample to the device, dropping what does not fit.
    pub(super) fn forward(&mut self, gb: &mut GameBoy) {
        if self.sender.is_empty() {
            for _ in 0..self.prefill_samples {
                let _ = self.sender.try_send(0.0);
            }
        }
        let mut correction = self.correction();
        loop {
            let count = gb.drain_audio(&mut self.scratch);
            for &sample in self.scratch.iter().take(count) {
                let repeats = match std::mem::replace(&mut correction, Correction::Keep) {
                    Correction::Keep => 1,
                    Correction::Drop => 0,
                    Correction::Repeat => 2,
                };
                for _ in 0..repeats {
                    // A full bridge means the device lags; dropping keeps latency bounded.
                    let _ = self.sender.try_send(sample);
                }
            }
            if count < SCRATCH_LEN_SAMPLES {
                return;
            }
        }
    }

    /// Returns whether the device disappeared and the output must be reopened.
    pub(super) fn is_lost(&self) -> bool {
        // Pairs with nothing: the flag carries no other data.
        self.lost.load(Ordering::Relaxed)
    }

    fn correction(&self) -> Correction {
        let target = usize::try_from(self.prefill_samples).unwrap_or(usize::MAX);
        correction(self.sender.len(), target)
    }
}

/// Change to one sample of a frame: ~0.1 % of pitch, inaudible, and enough to follow any
/// drift between the frame clock and the device clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Correction {
    Keep,
    Drop,
    Repeat,
}

/// Steers the queue towards its target, with a dead band of half of it.
fn correction(queued: usize, target: usize) -> Correction {
    let band = target / 2;
    if queued < target.saturating_sub(band) {
        Correction::Repeat
    } else if queued > target.saturating_add(band) {
        Correction::Drop
    } else {
        Correction::Keep
    }
}

fn build_stream<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    receiver: Receiver<f32>,
    lost: &Arc<AtomicBool>,
) -> Result<Stream, cpal::Error> {
    let channels = usize::from(config.channels);
    let lost = Arc::clone(lost);
    device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            fill_frames(data, channels, || receiver.try_recv().unwrap_or(0.0));
        },
        move |e| {
            eprintln!("Audio stream error: {e}");
            if ends_stream(e.kind()) {
                lost.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}

/// Returns whether only a new stream can restore the sound.
fn ends_stream(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated
    )
}

/// Writes one mono sample to every channel of each interleaved frame.
fn fill_frames<T: SizedSample + FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    mut next_sample: impl FnMut() -> f32,
) {
    for frame in data.chunks_mut(channels.max(1)) {
        frame.fill(T::from_sample(next_sample()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(channels: usize) -> [f32; 4] {
        let mut data = [0.0; 4];
        let mut samples = [0.125, 0.25, 0.5, 1.0].into_iter();
        fill_frames(&mut data, channels, || samples.next().unwrap_or(0.0));
        data
    }

    #[test]
    fn fill_frames_repeats_sample_across_channels() {
        for (channels, expected) in [
            (0, [0.125, 0.25, 0.5, 1.0]),
            (1, [0.125, 0.25, 0.5, 1.0]),
            (2, [0.125, 0.125, 0.25, 0.25]),
            (3, [0.125, 0.125, 0.125, 0.25]),
        ] {
            assert_eq!(
                filled(channels).map(f32::to_bits),
                expected.map(f32::to_bits)
            );
        }
    }

    #[test]
    fn display_names_the_cause() {
        for (error, text) in [
            (AudioError::NoDevice, "no output device"),
            (
                AudioError::UnsupportedFormat(SampleFormat::F64),
                "unsupported sample format f64",
            ),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    #[test]
    fn ends_stream_transient_errors_do_not() {
        for (kind, expected) in [
            (ErrorKind::DeviceNotAvailable, true),
            (ErrorKind::StreamInvalidated, true),
            (ErrorKind::Xrun, false),
            (ErrorKind::DeviceChanged, false),
        ] {
            assert_eq!(ends_stream(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn correction_steers_towards_target() {
        for (queued, expected) in [
            (0, Correction::Repeat),
            (749, Correction::Repeat),
            (750, Correction::Keep),
            (2250, Correction::Keep),
            (2251, Correction::Drop),
        ] {
            assert_eq!(correction(queued, 1500), expected, "{queued}");
        }
    }

    #[test]
    fn fill_frames_converts_sample_format() {
        let mut data = [0i16; 2];
        fill_frames(&mut data, 2, || 0.5);
        assert_eq!(data, [16_384; 2]);
    }
}
