use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tunic_engine::{OutputSinkError, ProcessedOutputFormat, ProcessedOutputSink};

const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = size_of::<f32>() as u16;
const SAMPLE_CAPACITY: usize = 1 << 18;
const WRITER_CHUNK_SAMPLES: usize = 8_192;

pub(crate) struct WavCapture {
    path: PathBuf,
    queue: Arc<SampleQueue>,
    finished: Arc<AtomicBool>,
    state: Mutex<CaptureState>,
}

struct CaptureState {
    configured: bool,
    failure: Option<String>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl WavCapture {
    pub(crate) fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            queue: Arc::new(SampleQueue::new(SAMPLE_CAPACITY)),
            finished: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(CaptureState {
                configured: false,
                failure: None,
                worker: None,
            }),
        }
    }

    pub(crate) fn finish(&self) -> Result<(), OutputSinkError> {
        self.finished.store(true, Ordering::Release);
        let (worker, failure) = {
            let mut state = self.state.lock().expect("capture state lock poisoned");
            (state.worker.take(), state.failure.take())
        };

        let writer_result = worker.map(|worker| {
            worker
                .join()
                .map_err(|_| OutputSinkError::new("capture writer thread panicked"))?
                .map_err(|error| OutputSinkError::new(format!("write audio capture: {error}")))
        });
        let dropped = self.queue.dropped.load(Ordering::Relaxed);

        if let Some(failure) = failure {
            Err(OutputSinkError::new(failure))
        } else if dropped != 0 {
            Err(OutputSinkError::new(format!(
                "audio capture dropped {dropped} samples"
            )))
        } else {
            writer_result.unwrap_or(Ok(()))
        }
    }
}

impl ProcessedOutputSink for WavCapture {
    fn configure(&self, format: ProcessedOutputFormat) -> Result<(), OutputSinkError> {
        let mut state = self.state.lock().expect("capture state lock poisoned");
        if state.configured {
            let message = "audio capture cannot continue after an output route change";
            state.failure = Some(message.into());
            return Err(OutputSinkError::new(message));
        }
        if format.channels != u32::from(CHANNELS) {
            return Err(OutputSinkError::new(format!(
                "audio capture requires stereo output, received {} channels",
                format.channels
            )));
        }
        let sample_rate = wav_sample_rate(format.sample_rate_hz)?;
        let file = File::create(&self.path).map_err(|error| {
            OutputSinkError::new(format!(
                "create capture file {}: {error}",
                self.path.display()
            ))
        })?;
        let queue = Arc::clone(&self.queue);
        let finished = Arc::clone(&self.finished);
        let worker = thread::Builder::new()
            .name("tunic-capture".into())
            .spawn(move || write_capture(file, sample_rate, &queue, &finished))
            .map_err(|error| OutputSinkError::new(format!("start capture writer: {error}")))?;
        state.configured = true;
        state.worker = Some(worker);
        Ok(())
    }

    fn write(&self, interleaved_samples: &[f32]) {
        self.queue.push(interleaved_samples);
    }
}

impl Drop for WavCapture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

struct SampleQueue {
    samples: Box<[AtomicU32]>,
    read: AtomicUsize,
    written: AtomicUsize,
    dropped: AtomicU64,
}

impl SampleQueue {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        let samples = (0..capacity)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            samples,
            read: AtomicUsize::new(0),
            written: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    fn push(&self, samples: &[f32]) {
        let written = self.written.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let available = self
            .samples
            .len()
            .saturating_sub(written.wrapping_sub(read));
        if samples.len() > available {
            self.dropped
                .fetch_add(samples.len() as u64, Ordering::Relaxed);
            return;
        }
        for (offset, sample) in samples.iter().enumerate() {
            self.samples[(written + offset) % self.samples.len()]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        self.written
            .store(written.wrapping_add(samples.len()), Ordering::Release);
    }

    fn pop(&self, output: &mut [u32]) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Acquire);
        let count = output.len().min(written.wrapping_sub(read));
        for (offset, sample) in output[..count].iter_mut().enumerate() {
            *sample = self.samples[(read + offset) % self.samples.len()].load(Ordering::Relaxed);
        }
        self.read.store(read.wrapping_add(count), Ordering::Release);
        count
    }

    fn is_empty(&self) -> bool {
        self.read.load(Ordering::Relaxed) == self.written.load(Ordering::Acquire)
    }
}

fn wav_sample_rate(sample_rate_hz: f64) -> Result<u32, OutputSinkError> {
    let rounded = sample_rate_hz.round();
    if sample_rate_hz.is_finite()
        && sample_rate_hz > 0.0
        && rounded <= u32::MAX as f64
        && sample_rate_hz == rounded
    {
        Ok(rounded as u32)
    } else {
        Err(OutputSinkError::new(format!(
            "cannot represent {sample_rate_hz} Hz in a WAV capture"
        )))
    }
}

fn write_capture(
    file: File,
    sample_rate: u32,
    queue: &SampleQueue,
    finished: &AtomicBool,
) -> io::Result<()> {
    let mut writer = BufWriter::new(file);
    write_wav_header(&mut writer, sample_rate, 0)?;
    let mut samples = [0_u32; WRITER_CHUNK_SAMPLES];
    let mut data_bytes = 0_u32;

    loop {
        let count = queue.pop(&mut samples);
        if count == 0 {
            if finished.load(Ordering::Acquire) && queue.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let bytes = u32::try_from(count * size_of::<f32>())
            .expect("capture writer chunk byte count fits in u32");
        data_bytes = data_bytes.checked_add(bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::FileTooLarge, "WAV capture exceeds 4 GiB")
        })?;
        for sample in &samples[..count] {
            writer.write_all(&sample.to_le_bytes())?;
        }
    }

    writer.seek(SeekFrom::Start(0))?;
    write_wav_header(&mut writer, sample_rate, data_bytes)?;
    writer.flush()
}

fn write_wav_header(writer: &mut impl Write, sample_rate: u32, data_bytes: u32) -> io::Result<()> {
    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let byte_rate = sample_rate * u32::from(block_align);
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36_u32 + data_bytes).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&3_u16.to_le_bytes())?;
    writer.write_all(&CHANNELS.to_le_bytes())?;
    writer.write_all(&sample_rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align.to_le_bytes())?;
    writer.write_all(&(BYTES_PER_SAMPLE * 8).to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tunic_engine::{ProcessedOutputFormat, ProcessedOutputSink};

    use super::{SampleQueue, WavCapture};

    #[test]
    fn writes_float32_stereo_wav_with_exact_samples() {
        let path = temporary_path("samples");
        let capture = WavCapture::new(&path);
        capture
            .configure(ProcessedOutputFormat {
                sample_rate_hz: 48_000.0,
                channels: 2,
            })
            .unwrap();
        let expected = [0.25_f32, -0.5, -0.75, 0.125];
        capture.write(&expected);
        capture.finish().unwrap();

        let bytes = fs::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
        let actual = bytes[44..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn full_queue_drops_the_entire_callback() {
        let queue = SampleQueue::new(4);
        queue.push(&[0.25, -0.5, -0.75, 0.125]);
        queue.push(&[1.0, -1.0]);
        let mut output = [0_u32; 4];

        assert_eq!(queue.pop(&mut output), 4);
        assert_eq!(queue.dropped.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(output.map(f32::from_bits), [0.25, -0.5, -0.75, 0.125]);
    }

    #[test]
    fn rejects_a_second_route_without_replacing_the_capture() {
        let path = temporary_path("route-change");
        let capture = WavCapture::new(&path);
        let format = ProcessedOutputFormat {
            sample_rate_hz: 44_100.0,
            channels: 2,
        };
        capture.configure(format).unwrap();

        assert!(capture.configure(format).is_err());
        assert!(capture.finish().is_err());
        fs::remove_file(path).unwrap();
    }

    fn temporary_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tunic-{name}-{}-{nonce}.wav", std::process::id()))
    }
}
