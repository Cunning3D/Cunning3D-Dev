//! Audio capture (microphone) and playback using cpal/rodio
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct AudioCapture {
    is_recording: Arc<AtomicBool>,
    samples_tx: Option<Sender<AudioChunk>>,
    samples_rx: Option<Receiver<AudioChunk>>,
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCapture {
    pub fn new() -> Self {
        let (tx, rx) = bounded(32);
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            samples_tx: Some(tx),
            samples_rx: Some(rx),
        }
    }

    pub fn start(&self) -> bool {
        if self.is_recording.swap(true, Ordering::SeqCst) {
            return false;
        }
        let is_rec = self.is_recording.clone();
        let tx = self.samples_tx.clone();
        std::thread::spawn(move || {
            #[cfg(feature = "voice_cpal")]
            Self::capture_loop(is_rec, tx);
            #[cfg(not(feature = "voice_cpal"))]
            {
                let _ = (is_rec, tx);
                eprintln!("[AudioCapture] cpal feature not enabled, skipping capture");
            }
        });
        true
    }

    pub fn stop(&self) {
        self.is_recording.store(false, Ordering::SeqCst);
    }

    pub fn take_receiver(&mut self) -> Option<Receiver<AudioChunk>> {
        self.samples_rx.take()
    }

    #[cfg(feature = "voice_cpal")]
    fn capture_loop(is_rec: Arc<AtomicBool>, tx: Option<Sender<AudioChunk>>) {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        use cpal::SampleFormat;
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                eprintln!("[AudioCapture] No input device");
                return;
            }
        };
        let config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[AudioCapture] Config error: {e}");
                return;
            }
        };
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let buffer: Arc<Mutex<Vec<f32>>> =
            Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize)));
        let buf_clone = buffer.clone();
        let stream_config: cpal::StreamConfig = config.clone().into();
        let err_fn = |e| eprintln!("[AudioCapture] Stream error: {e}");
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|c| c.iter().sum::<f32>() / channels as f32)
                        .collect();
                    if let Ok(mut b) = buf_clone.lock() {
                        b.extend(mono);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|c| {
                            c.iter()
                                .map(|&s| s as f32 / i16::MAX as f32)
                                .sum::<f32>()
                                / channels as f32
                        })
                        .collect();
                    if let Ok(mut b) = buf_clone.lock() {
                        b.extend(mono);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|c| {
                            c.iter()
                                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                                .sum::<f32>()
                                / channels as f32
                        })
                        .collect();
                    if let Ok(mut b) = buf_clone.lock() {
                        b.extend(mono);
                    }
                },
                err_fn,
                None,
            ),
            other => {
                eprintln!("[AudioCapture] Unsupported input sample format: {other:?}");
                return;
            }
        };
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[AudioCapture] Build stream error: {e}");
                return;
            }
        };
        let _ = stream.play();
        while is_rec.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(mut b) = buffer.lock() {
                if b.len() >= 1600 {
                    if let Some(ref tx) = tx {
                        let _ = tx.try_send(AudioChunk { samples: std::mem::take(&mut *b), sample_rate });
                    }
                }
            }
        }
    }
}

pub struct AudioPlayer;

impl AudioPlayer {
    pub fn play_file(path: &str) {
        #[cfg(feature = "voice_rodio")]
        {
            use rodio::{Decoder, OutputStream, Sink};
            use std::fs::File;
            use std::io::BufReader;
            let (_stream, handle) = match OutputStream::try_default() {
                Ok(s) => s,
                Err(_) => return,
            };
            let sink = match Sink::try_new(&handle) {
                Ok(s) => s,
                Err(_) => return,
            };
            let file = match File::open(path) {
                Ok(f) => f,
                Err(_) => return,
            };
            let source = match Decoder::new(BufReader::new(file)) {
                Ok(s) => s,
                Err(_) => return,
            };
            sink.append(source);
            sink.sleep_until_end();
        }
        #[cfg(not(feature = "voice_rodio"))]
        {
            let _ = path;
            eprintln!("[AudioPlayer] rodio feature not enabled");
        }
    }

    pub fn play_bytes(data: &[u8], sample_rate: u32) {
        #[cfg(feature = "voice_rodio")]
        {
            use rodio::{buffer::SamplesBuffer, OutputStream, Sink};
            let (_stream, handle) = match OutputStream::try_default() {
                Ok(s) => s,
                Err(_) => return,
            };
            let sink = match Sink::try_new(&handle) {
                Ok(s) => s,
                Err(_) => return,
            };
            let samples: Vec<i16> = data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            let source = SamplesBuffer::new(1, sample_rate, samples);
            sink.append(source);
            sink.sleep_until_end();
        }
        #[cfg(not(feature = "voice_rodio"))]
        {
            let _ = (data, sample_rate);
        }
    }
}
