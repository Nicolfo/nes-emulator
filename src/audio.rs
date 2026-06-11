//! Host audio output: a cpal stream fed from a shared sample queue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Cap the queue at ~340 ms @ 48 kHz so latency stays bounded if video stalls.
const MAX_BUFFER: usize = 16384;

pub struct Audio {
    pub sample_rate: u32,
    queue: Arc<Mutex<VecDeque<f32>>>,
    _stream: cpal::Stream,
}

impl Audio {
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let queue = Arc::new(Mutex::new(VecDeque::new()));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), channels, &queue),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), channels, &queue),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), channels, &queue),
            other => return Err(format!("unsupported sample format: {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;

        Ok(Audio {
            sample_rate,
            queue,
            _stream: stream,
        })
    }

    pub fn push(&self, samples: &[f32]) {
        let mut q = self.queue.lock().unwrap();
        q.extend(samples.iter().copied());
        let overflow = q.len().saturating_sub(MAX_BUFFER);
        if overflow > 0 {
            q.drain(..overflow);
        }
    }

    pub fn buffered(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let queue = queue.clone();
    let mut last = 0.0f32;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let mut q = queue.lock().unwrap();
                for frame in data.chunks_mut(channels) {
                    // on underrun, decay toward silence to avoid clicks
                    last = q.pop_front().unwrap_or(last * 0.995);
                    let v = T::from_sample(last);
                    for out in frame {
                        *out = v;
                    }
                }
            },
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
