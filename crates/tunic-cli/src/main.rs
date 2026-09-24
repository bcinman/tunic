//! Tunic's headless reference client.

mod capture;

use std::io::{self, BufRead, Write};
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
    ProcessedOutputSink,
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
    /// Configure the live equalizer.
    Filter {
        #[command(subcommand)]
        command: FilterCommand,
    },
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
        Command::Start { capture } => start_session(capture),
    }
}

fn start_session(capture_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let capture = capture_path
        .as_ref()
        .map(|path| Arc::new(WavCapture::new(path)));
    let output_sink = capture
        .as_ref()
        .map(|capture| Arc::clone(capture) as Arc<dyn ProcessedOutputSink>);
    let engine = Engine::start(
        EngineOptions {
            processed_output_sink: output_sink,
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
                if handle_line(&engine, &line)? {
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

fn handle_line(engine: &EngineHandle, line: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(words) = shlex::split(line) else {
        eprintln!("error: unmatched quote");
        return Ok(false);
    };
    if words.is_empty() {
        return Ok(false);
    }
    let command = match SessionCli::try_parse_from(words) {
        Ok(command) => command.command,
        Err(error) => {
            error.print()?;
            return Ok(false);
        }
    };

    match command {
        SessionCommand::Status => print_status(&engine.snapshot()),
        SessionCommand::Device { command } => match command {
            DeviceCommand::List => print_devices(&engine.snapshot()),
            DeviceCommand::Show { device } => show_device(&engine.snapshot(), device.as_deref()),
        },
        SessionCommand::Bypass => {
            let bypassed = engine.toggle_bypass()?;
            println!(
                "Processing is {}.",
                if bypassed { "bypassed" } else { "active" }
            );
        }
        SessionCommand::Filter { command } => {
            let snapshot = engine.snapshot();
            let equalizer = match equalizer_from_command(&snapshot.equalizer, command) {
                Ok(equalizer) => equalizer,
                Err(error) => {
                    eprintln!("error: {error}");
                    return Ok(false);
                }
            };
            match engine.set_equalizer(equalizer) {
                Ok(revision) => {
                    println!("Applied equalizer revision {}.", revision.get());
                }
                Err(error) => eprintln!("error: {error}"),
            }
        }
        SessionCommand::Help => print_session_help()?,
        SessionCommand::Quit => return Ok(true),
    }
    Ok(false)
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
            "Equalizer: identity (revision {})",
            snapshot.equalizer_revision.get()
        );
    } else {
        println!(
            "Equalizer: {} {} (revision {})",
            filters.len(),
            if filters.len() == 1 { "band" } else { "bands" },
            snapshot.equalizer_revision.get()
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
        DeviceCommand, FilterCommand, FilterKind, SessionCli, SessionCommand,
        equalizer_from_command,
    };
    use clap::Parser as _;
    use tunic_dsp::{Equalizer, Filter};

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
