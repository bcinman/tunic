//! Tunic's headless reference client.

mod capture;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};
use tunic_engine::{
    Engine, EngineHandle, EngineOptions, EngineSnapshot, EngineStatus, OutputDevice,
    ProcessedOutputSink, StereoLevels,
};
use tunic_macos::CoreAudioPlatform;

use crate::capture::WavCapture;

#[derive(Debug, Parser)]
#[command(name = "tunic", version, about = "Tunic's headless reference client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start processing system audio and open an interactive session.
    Start {
        /// Capture processed stereo output as a Float32 WAV file.
        #[arg(long, value_name = "PATH")]
        capture: Option<PathBuf>,
        /// Store Tunic state in this directory.
        #[arg(long, value_name = "DIRECTORY")]
        data_directory: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
#[command(name = "tunic", no_binary_name = true, disable_help_subcommand = true)]
struct SessionCli {
    #[command(subcommand)]
    command: SessionCommand,
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// Show the current engine and route state.
    Status,
    /// Inspect output devices.
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Toggle processing bypass.
    Bypass,
    /// Show the latest post-EQ stereo peak and RMS levels.
    Telemetry,
    /// Configure the live equalizer.
    Filter {
        #[command(subcommand)]
        command: FilterCommand,
    },
    /// Persist the current equalizer preview.
    Save,
    /// Restore the last persisted equalizer.
    Discard,
    /// Show interactive commands.
    Help,
    /// Shut down Tunic.
    Quit,
}

#[derive(Debug, Subcommand)]
enum DeviceCommand {
    /// List available output devices.
    List,
    /// Show one output device; defaults to the active device.
    Show { device: Option<String> },
}

#[derive(Debug, Subcommand)]
enum FilterCommand {
    /// Add an EQ band.
    Add {
        /// Filter type.
        kind: FilterKind,
        /// Center or shelf frequency in hertz.
        #[arg(long)]
        frequency: f64,
        /// Gain in decibels.
        #[arg(long, allow_hyphen_values = true)]
        gain: f64,
        /// Quality factor.
        #[arg(long)]
        q: f64,
    },
    /// Replace an EQ band.
    Set {
        /// One-based band number shown by `status`.
        band: usize,
        /// Filter type.
        kind: FilterKind,
        /// Center or shelf frequency in hertz.
        #[arg(long)]
        frequency: f64,
        /// Gain in decibels.
        #[arg(long, allow_hyphen_values = true)]
        gain: f64,
        /// Quality factor.
        #[arg(long)]
        q: f64,
    },
    /// Remove an EQ band.
    Remove {
        /// One-based band number shown by `status`.
        band: usize,
    },
    /// Remove all EQ bands and return to identity processing.
    Reset,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Start {
            capture,
            data_directory,
        } => start_session(capture, data_directory),
    }
}

fn start_session(
    capture_path: Option<PathBuf>,
    data_directory: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let data_directory = data_directory.map_or_else(default_data_directory, Ok)?;
    let capture = capture_path
        .as_ref()
        .map(|path| Arc::new(WavCapture::new(path)));
    let output_sink = capture
        .as_ref()
        .map(|capture| Arc::clone(capture) as Arc<dyn ProcessedOutputSink>);
    let engine = Engine::start(
        EngineOptions {
            processed_output_sink: output_sink,
            database_path: Some(data_directory.join("tunic.sqlite3")),
            ..EngineOptions::default()
        },
        CoreAudioPlatform::new,
    )?;
    println!("Tunic is processing system audio. Type `help` for commands.");
    if let Some(path) = capture_path {
        println!("Capturing processed output to {}.", path.display());
    }
    print_status(&engine.snapshot());

    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Release))?;

    let (lines, line_rx) = mpsc::channel();
    thread::Builder::new()
        .name("tunic-input".into())
        .spawn(move || read_input(lines))?;

    print_prompt()?;
    while !interrupted.load(Ordering::Acquire) {
        match line_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Some(line)) => {
                match handle_line(&engine, &line)? {
                    SessionAction::Continue => {}
                    SessionAction::StreamTelemetry => {
                        if stream_telemetry(&engine, &line_rx, &interrupted)? {
                            break;
                        }
                    }
                    SessionAction::Quit => break,
                }
                if interrupted.load(Ordering::Acquire) {
                    break;
                }
                print_prompt()?;
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    let shutdown = engine.shutdown();
    let capture_result = capture.map_or(Ok(()), |capture| capture.finish());
    shutdown?;
    capture_result?;
    println!("Tunic stopped.");
    Ok(())
}

fn default_data_directory() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is not set; pass --data-directory",
        )
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Tunic"))
}

fn read_input(lines: mpsc::Sender<Option<String>>) {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) => {
                if lines.send(Some(line)).is_err() {
                    return;
                }
            }
            Err(_) => break,
        }
    }
    let _ = lines.send(None);
}

enum SessionAction {
    Continue,
    StreamTelemetry,
    Quit,
}

fn handle_line(
    engine: &EngineHandle,
    line: &str,
) -> Result<SessionAction, Box<dyn std::error::Error>> {
    let Some(words) = shlex::split(line) else {
        eprintln!("error: unmatched quote");
        return Ok(SessionAction::Continue);
    };
    if words.is_empty() {
        return Ok(SessionAction::Continue);
    }
    let command = match SessionCli::try_parse_from(words) {
        Ok(command) => command.command,
        Err(error) => {
            error.print()?;
            return Ok(SessionAction::Continue);
        }
    };

    let action = match command {
        SessionCommand::Status => {
            print_status(&engine.snapshot());
            SessionAction::Continue
        }
        SessionCommand::Device { command } => {
            match command {
                DeviceCommand::List => print_devices(&engine.snapshot()),
                DeviceCommand::Show { device } => {
                    show_device(&engine.snapshot(), device.as_deref());
                }
            }
            SessionAction::Continue
        }
        SessionCommand::Bypass => {
            let bypassed = engine.toggle_bypass()?;
            println!(
                "Processing is {}.",
                if bypassed { "bypassed" } else { "active" }
            );
            SessionAction::Continue
        }
        SessionCommand::Telemetry => SessionAction::StreamTelemetry,
        SessionCommand::Filter { command } => {
            let snapshot = engine.snapshot();
            let equalizer = match equalizer_from_command(&snapshot.equalizer, command) {
                Ok(equalizer) => equalizer,
                Err(error) => {
                    eprintln!("error: {error}");
                    return Ok(SessionAction::Continue);
                }
            };
            match engine.preview_equalizer(equalizer, snapshot.edit_revision) {
                Ok(revision) => {
                    println!("Previewing equalizer edit revision {}.", revision.get());
                }
                Err(error) => eprintln!("error: {error}"),
            }
            SessionAction::Continue
        }
        SessionCommand::Save => {
            let snapshot = engine.snapshot();
            match engine.save_equalizer(snapshot.edit_revision) {
                Ok(revision) if snapshot.has_unsaved_changes => {
                    println!("Saved equalizer revision {}.", revision.get());
                }
                Ok(_) => println!("No equalizer changes to save."),
                Err(error) => eprintln!("error: {error}"),
            }
            SessionAction::Continue
        }
        SessionCommand::Discard => {
            let snapshot = engine.snapshot();
            match engine.discard_preview(snapshot.edit_revision) {
                Ok(revision) if snapshot.has_unsaved_changes => {
                    println!("Discarded preview at edit revision {}.", revision.get());
                }
                Ok(_) => println!("No equalizer changes to discard."),
                Err(error) => eprintln!("error: {error}"),
            }
            SessionAction::Continue
        }
        SessionCommand::Help => {
            print_session_help()?;
            SessionAction::Continue
        }
        SessionCommand::Quit => SessionAction::Quit,
    };
    Ok(action)
}

fn equalizer_from_command(
    current: &Equalizer,
    command: FilterCommand,
) -> Result<Equalizer, String> {
    let mut filters = current.filters().to_vec();
    match command {
        FilterCommand::Add {
            kind,
            frequency,
            gain,
            q,
        } => {
            filters.push(filter(kind, frequency, gain, q)?);
        }
        FilterCommand::Set {
            band,
            kind,
            frequency,
            gain,
            q,
        } => {
            let filter = filter(kind, frequency, gain, q)?;
            let Some(existing) = band.checked_sub(1).and_then(|index| filters.get_mut(index))
            else {
                return Err(format!("filter band {band} does not exist"));
            };
            *existing = filter;
        }
        FilterCommand::Remove { band } => {
            let Some(index) = band.checked_sub(1).filter(|&index| index < filters.len()) else {
                return Err(format!("filter band {band} does not exist"));
            };
            filters.remove(index);
        }
        FilterCommand::Reset => filters.clear(),
    }
    Ok(Equalizer::with_filters(filters))
}

fn filter(kind: FilterKind, frequency: f64, gain: f64, q: f64) -> Result<Filter, String> {
    let frequency = FrequencyHz::new(frequency).map_err(|error| error.to_string())?;
    let gain = GainDb::new(gain).map_err(|error| error.to_string())?;
    let quality_factor = QualityFactor::new(q).map_err(|error| error.to_string())?;
    Ok(match kind {
        FilterKind::Peaking => Filter::peaking(frequency, gain, quality_factor),
        FilterKind::LowShelf => Filter::low_shelf(frequency, gain, quality_factor),
        FilterKind::HighShelf => Filter::high_shelf(frequency, gain, quality_factor),
    })
}

fn print_status(snapshot: &EngineSnapshot) {
    let status = match &snapshot.status {
        EngineStatus::Starting => "starting".to_owned(),
        EngineStatus::Running => "running".to_owned(),
        EngineStatus::Failed(error) => format!("failed: {error}"),
        EngineStatus::Stopped => "stopped".to_owned(),
    };
    println!("Status: {status}");
    println!(
        "Processing: {}",
        if snapshot.bypassed {
            "bypassed"
        } else {
            "active"
        }
    );
    if let Some(route) = &snapshot.route {
        println!(
            "Output: {} ({} Hz, {} channels)",
            route.device_name, route.sample_rate_hz, route.channels
        );
    } else {
        println!("Output: none");
    }
    let filters = snapshot.equalizer.filters();
    if filters.is_empty() {
        println!(
            "Equalizer: identity (saved revision {}, edit revision {}{})",
            snapshot.equalizer_revision.get(),
            snapshot.edit_revision.get(),
            if snapshot.has_unsaved_changes {
                ", unsaved"
            } else {
                ""
            }
        );
    } else {
        println!(
            "Equalizer: {} {} (saved revision {}, edit revision {}{})",
            filters.len(),
            if filters.len() == 1 { "band" } else { "bands" },
            snapshot.equalizer_revision.get(),
            snapshot.edit_revision.get(),
            if snapshot.has_unsaved_changes {
                ", unsaved"
            } else {
                ""
            }
        );
        for (index, filter) in filters.iter().enumerate() {
            let kind = match filter {
                Filter::Peaking { .. } => "peaking",
                Filter::LowShelf { .. } => "low-shelf",
                Filter::HighShelf { .. } => "high-shelf",
            };
            println!(
                "  {}: {kind}, {} Hz, {:+} dB, Q {}",
                index + 1,
                filter.frequency().get(),
                filter.gain().get(),
                filter.quality_factor().get()
            );
        }
    }
}

fn print_devices(snapshot: &EngineSnapshot) {
    for device in &snapshot.devices {
        let active = snapshot
            .route
            .as_ref()
            .is_some_and(|route| route.device_id == device.id);
        let marker = match (device.is_default, active) {
            (true, true) => "default, active",
            (true, false) => "default",
            (false, true) => "active",
            (false, false) => "",
        };
        if marker.is_empty() {
            println!("  {}", device.name);
        } else {
            println!("* {} ({marker})", device.name);
        }
    }
}

fn show_device(snapshot: &EngineSnapshot, query: Option<&str>) {
    let device = query.map_or_else(
        || {
            snapshot.route.as_ref().and_then(|route| {
                snapshot
                    .devices
                    .iter()
                    .find(|device| device.id == route.device_id)
            })
        },
        |query| find_device(&snapshot.devices, query),
    );
    let Some(device) = device else {
        eprintln!("error: output device not found");
        return;
    };
    println!("Name: {}", device.name);
    println!("ID: {}", device.id);
    println!("Sample rate: {} Hz", device.sample_rate_hz);
    println!("Channels: {}", device.channels);
    println!("System default: {}", device.is_default);
}

fn find_device<'a>(devices: &'a [OutputDevice], query: &str) -> Option<&'a OutputDevice> {
    devices
        .iter()
        .find(|device| device.id.as_str() == query || device.name == query)
}

fn stream_telemetry(
    engine: &EngineHandle,
    lines: &mpsc::Receiver<Option<String>>,
    interrupted: &AtomicBool,
) -> io::Result<bool> {
    let telemetry = engine.telemetry();
    let is_terminal = io::stdout().is_terminal();
    let mut levels = telemetry.try_latest().unwrap_or_default();
    println!("Live telemetry (press Enter to stop)");
    if is_terminal {
        print!("\x1b[?25l\x1b7");
    }

    let disconnected = loop {
        if let Some(latest) = telemetry.try_latest() {
            levels = latest;
        }
        let formatted = format_telemetry(levels);
        if is_terminal {
            print!("\x1b8\x1b[J{formatted}");
        } else {
            println!("{formatted}");
        }
        io::stdout().flush()?;

        if interrupted.load(Ordering::Acquire) {
            break false;
        }
        match lines.recv_timeout(Duration::from_millis(33)) {
            Ok(Some(_)) => break false,
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };

    if is_terminal {
        print!("\x1b8\x1b[J\x1b[?25h");
    }
    io::stdout().flush()?;
    Ok(disconnected)
}

fn format_telemetry(levels: StereoLevels) -> String {
    format!(
        "L [{}] peak {:>6.1} dBFS  RMS {:>6.1} dBFS\nR [{}] peak {:>6.1} dBFS  RMS {:>6.1} dBFS",
        level_bar(levels.left.peak),
        decibels_full_scale(levels.left.peak),
        decibels_full_scale(levels.left.rms),
        level_bar(levels.right.peak),
        decibels_full_scale(levels.right.peak),
        decibels_full_scale(levels.right.rms),
    )
}

fn level_bar(amplitude: f32) -> String {
    const WIDTH: usize = 20;
    let normalized = ((decibels_full_scale(amplitude) + 60.0) / 60.0).clamp(0.0, 1.0);
    let filled = (normalized * WIDTH as f32).round() as usize;
    format!("{}{}", "#".repeat(filled), "-".repeat(WIDTH - filled))
}

fn decibels_full_scale(amplitude: f32) -> f32 {
    if amplitude > 0.0 {
        20.0 * amplitude.log10()
    } else {
        -120.0
    }
}

fn print_session_help() -> Result<(), clap::Error> {
    SessionCli::command().print_help()?;
    println!();
    Ok(())
}

fn print_prompt() -> io::Result<()> {
    print!("tunic> ");
    io::stdout().flush()
}

use clap::CommandFactory as _;

#[cfg(test)]
mod tests {
    use super::{
        Cli, Command, DeviceCommand, FilterCommand, FilterKind, SessionCli, SessionCommand,
        equalizer_from_command, format_telemetry,
    };
    use clap::Parser as _;
    use tunic_dsp::{Equalizer, Filter};
    use tunic_engine::{ChannelLevels, StereoLevels};

    #[test]
    fn formats_asymmetric_linear_levels_as_dbfs() {
        let output = format_telemetry(StereoLevels {
            left: ChannelLevels {
                peak: 1.0,
                rms: 0.5,
            },
            right: ChannelLevels {
                peak: 0.25,
                rms: 0.0,
            },
        });

        assert!(output.contains("peak    0.0 dBFS  RMS   -6.0 dBFS"));
        assert!(output.contains("peak  -12.0 dBFS  RMS -120.0 dBFS"));
    }

    #[test]
    fn parses_a_custom_data_directory() {
        let parsed =
            Cli::try_parse_from(["tunic", "start", "--data-directory", "/tmp/tunic-test"]).unwrap();

        assert!(matches!(
            parsed.command,
            Command::Start {
                data_directory: Some(path),
                ..
            } if path == std::path::Path::new("/tmp/tunic-test")
        ));
    }

    #[test]
    fn interactive_device_name_preserves_spaces() {
        let words = shlex::split("device show \"Studio Display Speakers\"").unwrap();
        let parsed = SessionCli::try_parse_from(words).unwrap();

        assert!(matches!(
            parsed.command,
            SessionCommand::Device {
                command: DeviceCommand::Show { device: Some(name) }
            } if name == "Studio Display Speakers"
        ));
    }

    #[test]
    fn parses_peaking_filter_with_negative_gain() {
        let words = shlex::split("filter add peaking --frequency 1000 --gain -6 --q 1.25").unwrap();
        let parsed = SessionCli::try_parse_from(words).unwrap();

        assert!(matches!(
            parsed.command,
            SessionCommand::Filter {
                command: FilterCommand::Add {
                    kind: FilterKind::Peaking,
                    frequency: 1000.0,
                    gain: -6.0,
                    q: 1.25,
                }
            }
        ));
    }

    #[test]
    fn rejects_invalid_filter_parameters_without_starting_the_engine() {
        let result = equalizer_from_command(
            &Equalizer::identity(),
            FilterCommand::Add {
                kind: FilterKind::LowShelf,
                frequency: 0.0,
                gain: 6.0,
                q: 1.0,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn add_set_remove_and_reset_build_an_ordered_equalizer() {
        let first = equalizer_from_command(
            &Equalizer::identity(),
            FilterCommand::Add {
                kind: FilterKind::LowShelf,
                frequency: 100.0,
                gain: 3.0,
                q: 0.7,
            },
        )
        .unwrap();
        let second = equalizer_from_command(
            &first,
            FilterCommand::Add {
                kind: FilterKind::HighShelf,
                frequency: 1_000.0,
                gain: -4.0,
                q: 1.5,
            },
        )
        .unwrap();
        let changed = equalizer_from_command(
            &second,
            FilterCommand::Set {
                band: 1,
                kind: FilterKind::Peaking,
                frequency: 200.0,
                gain: 6.0,
                q: 1.0,
            },
        )
        .unwrap();

        assert_eq!(changed.filters().len(), 2);
        assert!(matches!(changed.filters()[0], Filter::Peaking { .. }));
        assert_eq!(changed.filters()[0].frequency().get(), 200.0);
        assert!(matches!(changed.filters()[1], Filter::HighShelf { .. }));
        assert_eq!(changed.filters()[1].frequency().get(), 1_000.0);

        let removed = equalizer_from_command(&changed, FilterCommand::Remove { band: 1 }).unwrap();
        assert_eq!(removed.filters().len(), 1);
        assert_eq!(removed.filters()[0].frequency().get(), 1_000.0);

        let reset = equalizer_from_command(&removed, FilterCommand::Reset).unwrap();
        assert!(reset.filters().is_empty());
    }

    #[test]
    fn rejects_zero_and_out_of_range_band_numbers() {
        let equalizer = equalizer_from_command(
            &Equalizer::identity(),
            FilterCommand::Add {
                kind: FilterKind::Peaking,
                frequency: 100.0,
                gain: 3.0,
                q: 1.0,
            },
        )
        .unwrap();

        assert!(equalizer_from_command(&equalizer, FilterCommand::Remove { band: 0 }).is_err());
        assert!(equalizer_from_command(&equalizer, FilterCommand::Remove { band: 2 }).is_err());
    }
}
