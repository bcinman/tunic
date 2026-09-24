#![cfg(target_os = "macos")]

use std::f32::consts::TAU;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

const SAMPLE_RATE: u32 = 48_000;
const LEFT_HZ: f32 = 101.0;
const RIGHT_HZ: f32 = 9_997.0;
const SHELF_HZ: f32 = 1_000.0;
const EXPECTED_CONTRAST_DB: f32 = 6.0;
const SHELF_GAIN_DB: f32 = EXPECTED_CONTRAST_DB / 2.0;

#[test]
#[ignore = "uses the current macOS output device"]
fn saved_filters_and_telemetry_work_through_the_production_route() {
    let artifacts = Artifacts::new();
    write_probe(&artifacts.probe);

    let mut tunic = Command::new(env!("CARGO_BIN_EXE_tunic"))
        .arg("start")
        .arg("--data-directory")
        .arg(&artifacts.data_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start Tunic");
    let mut output = BufReader::new(tunic.stdout.take().expect("capture Tunic stdout"));
    wait_for_output(&mut output, "Tunic is processing system audio");
    let mut input = tunic.stdin.take().expect("capture Tunic stdin");
    writeln!(
        input,
        "filter add low-shelf --frequency {SHELF_HZ} --gain {SHELF_GAIN_DB} --q 1"
    )
    .expect("configure low-shelf filter");
    wait_for_output(&mut output, "Previewing equalizer edit revision 1");
    writeln!(
        input,
        "filter add high-shelf --frequency {SHELF_HZ} --gain -{SHELF_GAIN_DB} --q 1"
    )
    .expect("configure high-shelf filter");
    wait_for_output(&mut output, "Previewing equalizer edit revision 2");
    input.write_all(b"save\n").expect("save equalizer");
    wait_for_output(&mut output, "Saved equalizer revision 1");
    input.write_all(b"quit\n").expect("stop Tunic");
    let status = tunic.wait().expect("wait for Tunic");
    assert!(status.success(), "Tunic failed with {status}");

    let mut tunic = Command::new(env!("CARGO_BIN_EXE_tunic"))
        .args(["start", "--capture"])
        .arg(&artifacts.capture)
        .arg("--data-directory")
        .arg(&artifacts.data_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("restart Tunic");
    let mut output = BufReader::new(tunic.stdout.take().expect("capture Tunic stdout"));
    wait_for_output(&mut output, "Capturing processed output");
    wait_for_output(
        &mut output,
        "Equalizer: 2 bands (saved revision 1, edit revision 0)",
    );
    let mut input = tunic.stdin.take().expect("capture Tunic stdin");

    let mut playback = Command::new("/usr/bin/afplay")
        .arg(&artifacts.probe)
        .spawn()
        .expect("play stereo probe");
    thread::sleep(Duration::from_millis(250));
    input
        .write_all(b"telemetry\n")
        .expect("request live telemetry");
    let left_levels = read_until_output(&mut output, "L [");
    let right_levels = read_until_output(&mut output, "R [");
    assert!(!left_levels.contains("-120.0 dBFS"), "{left_levels}");
    assert!(!right_levels.contains("-120.0 dBFS"), "{right_levels}");
    read_until_output(&mut output, "L [");
    read_until_output(&mut output, "R [");
    input.write_all(b"\n").expect("stop live telemetry");

    let playback = playback.wait().expect("wait for stereo probe");
    assert!(playback.success(), "afplay failed with {playback}");

    input.write_all(b"quit\n").expect("stop Tunic");
    let status = tunic.wait().expect("wait for Tunic");
    assert!(status.success(), "Tunic failed with {status}");

    verify_capture(&artifacts.capture);
}

fn wait_for_output(output: &mut impl BufRead, expected: &str) {
    let _ = read_until_output(output, expected);
}

fn read_until_output(output: &mut impl BufRead, expected: &str) -> String {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = output.read_line(&mut line).expect("read Tunic startup");
        assert_ne!(bytes, 0, "Tunic stopped before capture started");
        if line.contains(expected) {
            return line;
        }
    }
}

fn write_probe(path: &Path) {
    let frames = SAMPLE_RATE as usize;
    let data_bytes = u32::try_from(frames * 2 * size_of::<f32>()).unwrap();
    let mut file = fs::File::create(path).expect("create stereo probe");
    write_wav_header(&mut file, data_bytes);
    for frame in 0..frames {
        let time = frame as f32 / SAMPLE_RATE as f32;
        let left = 0.1 * (TAU * LEFT_HZ * time).sin();
        let right = 0.1 * (TAU * RIGHT_HZ * time).sin();
        file.write_all(&left.to_le_bytes()).unwrap();
        file.write_all(&right.to_le_bytes()).unwrap();
    }
}

fn write_wav_header(file: &mut impl Write, data_bytes: u32) {
    let channels = 2_u16;
    let bytes_per_sample = size_of::<f32>() as u16;
    let block_align = channels * bytes_per_sample;
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&3_u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    file.write_all(&(SAMPLE_RATE * u32::from(block_align)).to_le_bytes())
        .unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&(bytes_per_sample * 8).to_le_bytes())
        .unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_bytes.to_le_bytes()).unwrap();
}

fn verify_capture(path: &Path) {
    let bytes = fs::read(path).expect("read Tunic capture");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let capture_sample_rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());

    let samples = bytes[44..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    let left = samples.iter().step_by(2).copied().collect::<Vec<_>>();
    let right = samples
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .collect::<Vec<_>>();
    let active = (0..left.len())
        .filter(|&frame| left[frame].abs() > 0.01 || right[frame].abs() > 0.01)
        .collect::<Vec<_>>();
    assert!(!active.is_empty(), "capture contains no probe signal");

    let first = active[0] + 1_000;
    let last = active[active.len() - 1] - 1_000;
    assert!(last > first, "captured probe is too short");
    let duration = (last - first) as f32 / capture_sample_rate as f32;
    let left_frequency = positive_crossings(&left, first, last) as f32 / duration;
    let right_frequency = positive_crossings(&right, first, last) as f32 / duration;
    let left_rms = rms(&left[first..=last]);
    let right_rms = rms(&right[first..=last]);
    let measured_contrast = left_rms / right_rms;
    let expected_contrast = 10.0_f32.powf(EXPECTED_CONTRAST_DB / 20.0);

    println!(
        "captured at {capture_sample_rate} Hz: left={left_frequency:.1} Hz, right={right_frequency:.1} Hz, measured low/high gain contrast={measured_contrast:.2}"
    );
    assert!((left_frequency - LEFT_HZ).abs() < 3.0);
    assert!((right_frequency - RIGHT_HZ).abs() < 3.0);
    assert!((measured_contrast - expected_contrast).abs() < 0.1);
}

fn positive_crossings(samples: &[f32], first: usize, last: usize) -> usize {
    (first..last)
        .filter(|&frame| samples[frame] <= 0.0 && samples[frame + 1] > 0.0)
        .count()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

struct Artifacts {
    probe: PathBuf,
    capture: PathBuf,
    data_directory: PathBuf,
}

impl Artifacts {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let stem = format!("tunic-hardware-{}-{nonce}", std::process::id());
        Self {
            probe: std::env::temp_dir().join(format!("{stem}-probe.wav")),
            capture: std::env::temp_dir().join(format!("{stem}-capture.wav")),
            data_directory: std::env::temp_dir().join(format!("{stem}-data")),
        }
    }
}

impl Drop for Artifacts {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.probe);
        let _ = fs::remove_file(&self.capture);
        let _ = fs::remove_dir_all(&self.data_directory);
    }
}
