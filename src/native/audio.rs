//! Bridge from the core's mono samples to the default audio device.
//!
//! The emulation thread pushes into a bounded channel; the device callback pops without
//! blocking and plays silence when it runs dry.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};
use rusty_gb::GameBoy;
use std::error::Error;

/// Bridge capacity as a fraction of a second (125 ms), the latency bound.
const BRIDGE_FRACTION: u32 = 8;
/// Silence queued after running dry (~31 ms), so one late frame is not one more gap.
const PREFILL_FRACTION: u32 = 32;
const SCRATCH_LEN_SAMPLES: usize = 1024;

pub(super) struct AudioOutput {
    /// Playback stops when the stream is dropped.
    _stream: Stream,
    sender: Sender<f32>,
    sample_rate_hz: u32,
    prefill_samples: u32,
    scratch: [f32; SCRATCH_LEN_SAMPLES],
}

impl AudioOutput {
    pub(super) fn open() -> Result<Self, Box<dyn Error>> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no output device")?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let sample_rate_hz = config.sample_rate.0;
        let (sender, receiver) = bounded(usize::try_from(sample_rate_hz / BRIDGE_FRACTION)?);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, receiver),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, receiver),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, receiver),
            SampleFormat::I32 => build_stream::<i32>(&device, &config, receiver),
            SampleFormat::U8 => build_stream::<u8>(&device, &config, receiver),
            other => return Err(format!("unsupported sample format {other}").into()),
        }?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            sender,
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
        loop {
            let count = gb.drain_audio(&mut self.scratch);
            for &sample in self.scratch.iter().take(count) {
                // A full bridge means the device lags; dropping keeps latency bounded.
                let _ = self.sender.try_send(sample);
            }
            if count < SCRATCH_LEN_SAMPLES {
                return;
            }
        }
    }
}

fn build_stream<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    receiver: Receiver<f32>,
) -> Result<Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels);
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            fill_frames(data, channels, || receiver.try_recv().unwrap_or(0.0));
        },
        |e| eprintln!("Audio stream error: {e}"),
        None,
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
    fn fill_frames_converts_sample_format() {
        let mut data = [0i16; 2];
        fill_frames(&mut data, 2, || 0.5);
        assert_eq!(data, [16_384; 2]);
    }
}
