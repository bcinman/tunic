//! Tunic's headless reference client.

mod capture;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};
use tunic_engine::{
    Engine, EngineHandle, EngineOptions, EngineSnapshot, EngineStatus, OutputDevice,
    ProcessedOutputSink, Profile, ProfileId, SPECTRUM_BAND_COUNT, Spectrum, TelemetryFrame,
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
    /// Process system audio and print every engine state change.
    Watch {
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
    /// Manage equalizer profiles.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Toggle processing bypass.
    Bypass,
    /// Show post-EQ stereo levels and a live spectrum analyzer.
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
    /// Assign a profile to an output device; defaults to the active device.
    SetProfile {
        profile: String,
        device: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// List profiles.
    List,
    /// Show one profile.
    Info { profile: String },
    /// Create and activate a profile from the current equalizer.
    Create { name: String },
    /// Rename a profile.
    Rename { profile: String, name: String },
    /// Delete a profile.
    Delete { profile: String },
    /// Activate a profile and make it the default.
    Select { profile: String },
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
        Command::Watch { data_directory } => watch(data_directory),
    }
}

fn start_session(
    capture_path: Option<PathBuf>,
    data_directory: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let capture = capture_path
        .as_ref()
        .map(|path| Arc::new(WavCapture::new(path)));
    let output_sink = capture
        .as_ref()
        .map(|capture| Arc::clone(capture) as Arc<dyn ProcessedOutputSink>);
    let engine = start_engine(data_directory, output_sink)?;
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

fn watch(data_directory: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let engine = start_engine(data_directory, None)?;
    let snapshots = engine.subscribe_snapshots();
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Release))?;
    let mut first = true;

    while !interrupted.load(Ordering::Acquire) {
        match snapshots.recv_timeout(Duration::from_millis(50)) {
            Ok(snapshot) => print_watched_snapshot(&snapshot, &mut first)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let shutdown = engine.shutdown();
    while let Ok(snapshot) = snapshots.recv() {
        print_watched_snapshot(&snapshot, &mut first)?;
    }
    shutdown?;
    Ok(())
}

fn print_watched_snapshot(snapshot: &EngineSnapshot, first: &mut bool) -> io::Result<()> {
    if !*first {
        println!("---");
    }
    print_status(snapshot);
    io::stdout().flush()?;
    *first = false;
    Ok(())
}

fn start_engine(
    data_directory: Option<PathBuf>,
    output_sink: Option<Arc<dyn ProcessedOutputSink>>,
) -> Result<EngineHandle, Box<dyn std::error::Error>> {
    let data_directory = data_directory.map_or_else(default_data_directory, Ok)?;
    Ok(Engine::start(
        EngineOptions {
            processed_output_sink: output_sink,
            database_path: Some(data_directory.join("tunic.sqlite3")),
            ..EngineOptions::default()
        },
        CoreAudioPlatform::new,
    )?)
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
                DeviceCommand::SetProfile { profile, device } => {
                    let snapshot = engine.snapshot();
                    let profile_id = match find_profile_id(&snapshot, &profile) {
                        Ok(profile_id) => profile_id,
                        Err(error) => {
                            eprintln!("error: {error}");
                            return Ok(SessionAction::Continue);
                        }
                    };
                    let output = device.as_deref().map_or_else(
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
                    let Some(output) = output else {
                        eprintln!("error: output device not found");
                        return Ok(SessionAction::Continue);
                    };
                    let output_name = output.name.clone();
                    match engine.assign_profile(output.id.clone(), profile_id) {
                        Ok(()) => println!("Assigned profile to {output_name}."),
                        Err(error) => eprintln!("error: {error}"),
                    }
                }
            }
            SessionAction::Continue
        }
        SessionCommand::Profile { command } => {
            let snapshot = engine.snapshot();
            match command {
                ProfileCommand::List => print_profiles(&snapshot),
                ProfileCommand::Info { profile } => match find_profile(&snapshot, &profile) {
                    Ok(profile) => print_profile(&snapshot, profile),
                    Err(error) => eprintln!("error: {error}"),
                },
                ProfileCommand::Create { name } => match engine.create_profile(name) {
                    Ok(profile_id) => println!("Created and selected profile {profile_id}."),
                    Err(error) => eprintln!("error: {error}"),
                },
                ProfileCommand::Rename { profile, name } => {
                    match find_profile_id(&snapshot, &profile) {
                        Ok(profile_id) => match engine.rename_profile(profile_id, name) {
                            Ok(()) => println!("Renamed profile."),
                            Err(error) => eprintln!("error: {error}"),
                        },
                        Err(error) => eprintln!("error: {error}"),
                    }
                }
                ProfileCommand::Delete { profile } => match find_profile_id(&snapshot, &profile) {
                    Ok(profile_id) => match engine.delete_profile(profile_id) {
                        Ok(()) => println!("Deleted profile."),
                        Err(error) => eprintln!("error: {error}"),
                    },
                    Err(error) => eprintln!("error: {error}"),
                },
                ProfileCommand::Select { profile } => match find_profile_id(&snapshot, &profile) {
                    Ok(profile_id) => match engine.select_profile(profile_id) {
                        Ok(()) => println!("Selected profile."),
                        Err(error) => eprintln!("error: {error}"),
                    },
                    Err(error) => eprintln!("error: {error}"),
                },
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
    if let Some(profile) = snapshot
        .active_profile_id
        .as_ref()
        .and_then(|id| snapshot.profiles.iter().find(|profile| &profile.id == id))
    {
        println!("Profile: {} ({})", profile.name, profile.id);
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
    let profile_id = snapshot
        .device_profile_assignments
        .iter()
        .find(|assignment| assignment.device_id == device.id)
        .map(|assignment| &assignment.profile_id)
        .or(snapshot.default_profile_id.as_ref());
    if let Some(profile) = profile_id.and_then(|profile_id| {
        snapshot
            .profiles
            .iter()
            .find(|profile| &profile.id == profile_id)
    }) {
        println!("Profile: {} ({})", profile.name, profile.id);
    }
}

fn find_device<'a>(devices: &'a [OutputDevice], query: &str) -> Option<&'a OutputDevice> {
    devices
        .iter()
        .find(|device| device.id.as_str() == query || device.name == query)
}

fn find_profile<'a>(snapshot: &'a EngineSnapshot, query: &str) -> Result<&'a Profile, String> {
    let profiles = snapshot
        .profiles
        .iter()
        .map(|profile| (&profile.id, profile.name.as_str()));
    let profile_id = matching_profile_id(profiles, query)
        .ok_or_else(|| format!("profile '{query}' not found"))?;
    Ok(snapshot
        .profiles
        .iter()
        .find(|profile| &profile.id == profile_id)
        .expect("matched profile id came from the snapshot"))
}

fn matching_profile_id<'a, I>(mut profiles: I, query: &str) -> Option<&'a ProfileId>
where
    I: Clone + Iterator<Item = (&'a ProfileId, &'a str)>,
{
    profiles
        .clone()
        .find_map(|(id, _)| (id.as_str() == query).then_some(id))
        .or_else(|| profiles.find_map(|(id, name)| name.eq_ignore_ascii_case(query).then_some(id)))
}

fn find_profile_id(snapshot: &EngineSnapshot, query: &str) -> Result<ProfileId, String> {
    find_profile(snapshot, query).map(|profile| profile.id.clone())
}

fn print_profiles(snapshot: &EngineSnapshot) {
    for profile in &snapshot.profiles {
        let active = snapshot.active_profile_id.as_ref() == Some(&profile.id);
        let default = snapshot.default_profile_id.as_ref() == Some(&profile.id);
        let marker = match (active, default) {
            (true, true) => "active, default",
            (true, false) => "active",
            (false, true) => "default",
            (false, false) => "",
        };
        if marker.is_empty() {
            println!("  {} ({})", profile.name, profile.id);
        } else {
            println!("* {} ({}, {marker})", profile.name, profile.id);
        }
    }
}

fn print_profile(snapshot: &EngineSnapshot, profile: &Profile) {
    println!("Name: {}", profile.name);
    println!("ID: {}", profile.id);
    println!("Revision: {}", profile.revision.get());
    println!(
        "Active: {}",
        snapshot.active_profile_id.as_ref() == Some(&profile.id)
    );
    println!(
        "Default: {}",
        snapshot.default_profile_id.as_ref() == Some(&profile.id)
    );
    println!("Bands: {}", profile.equalizer.filters().len());
}

fn stream_telemetry(
    engine: &EngineHandle,
    lines: &mpsc::Receiver<Option<String>>,
    interrupted: &AtomicBool,
) -> io::Result<bool> {
    let telemetry = engine.telemetry();
    let is_terminal = io::stdout().is_terminal();
    let mut frame = telemetry.try_latest().unwrap_or_default();
    let mut terminal_size = is_terminal.then(read_terminal_size).flatten();
    let mut terminal_size_read_at = Instant::now();
    let screen = is_terminal.then(TerminalScreen::enter).transpose()?;
    let mut output = io::stdout().lock();
    if !is_terminal {
        writeln!(output, "Live telemetry (press Enter to stop)")?;
    }

    let disconnected = loop {
        if let Some(latest) = telemetry.try_latest() {
            frame = latest;
        }
        if is_terminal {
            if terminal_size_read_at.elapsed() >= Duration::from_millis(500) {
                terminal_size = read_terminal_size();
                terminal_size_read_at = Instant::now();
            }
            write!(
                output,
                "\x1b[H{}\x1b[J",
                format_terminal_telemetry(frame, terminal_size)
            )?;
        } else {
            writeln!(output, "{}", format_telemetry(frame))?;
        }
        output.flush()?;

        if interrupted.load(Ordering::Acquire) {
            break false;
        }
        match lines.recv_timeout(Duration::from_millis(33)) {
            Ok(Some(_)) => break false,
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };

    drop(output);
    drop(screen);
    Ok(disconnected)
}

const TELEMETRY_MINIMUM_ROWS: u16 = 18;
const TELEMETRY_MINIMUM_COLUMNS: u16 = 62;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalSize {
    rows: u16,
    columns: u16,
}

fn read_terminal_size() -> Option<TerminalSize> {
    let output = ProcessCommand::new("stty")
        .args(["-f", "/dev/tty", "size"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_terminal_size(&output.stdout))?
}

fn parse_terminal_size(output: &[u8]) -> Option<TerminalSize> {
    let mut dimensions = std::str::from_utf8(output).ok()?.split_whitespace();
    let rows = dimensions.next()?.parse().ok()?;
    let columns = dimensions.next()?.parse().ok()?;
    dimensions
        .next()
        .is_none()
        .then_some(TerminalSize { rows, columns })
}

struct TerminalScreen;

impl TerminalScreen {
    fn enter() -> io::Result<Self> {
        let screen = Self;
        let mut output = io::stdout().lock();
        if let Err(error) = output
            .write_all(b"\x1b[?1049h\x1b[?25l")
            .and_then(|()| output.flush())
        {
            drop(output);
            drop(screen);
            return Err(error);
        }
        Ok(screen)
    }
}

impl Drop for TerminalScreen {
    fn drop(&mut self) {
        let mut output = io::stdout().lock();
        let _ = output.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = output.flush();
    }
}

fn format_terminal_telemetry(frame: TelemetryFrame, size: Option<TerminalSize>) -> String {
    let Some(size) = size else {
        return "Live telemetry (press Enter to stop)\n\nTerminal size unavailable".into();
    };
    if size.rows < TELEMETRY_MINIMUM_ROWS || size.columns < TELEMETRY_MINIMUM_COLUMNS {
        return format!(
            "Live telemetry (press Enter to stop)\n\nTerminal too small: {}×{}\nMinimum size: {}×{}",
            size.columns, size.rows, TELEMETRY_MINIMUM_COLUMNS, TELEMETRY_MINIMUM_ROWS
        );
    }
    format!(
        "Live telemetry (press Enter to stop)\n\n{}",
        format_telemetry(frame)
    )
}

fn format_telemetry(frame: TelemetryFrame) -> String {
    let levels = frame.levels;
    let mut output = format!(
        "L [{}] peak {:>6.1} dBFS  RMS {:>6.1} dBFS\nR [{}] peak {:>6.1} dBFS  RMS {:>6.1} dBFS",
        level_bar(levels.left.peak),
        decibels_full_scale(levels.left.peak),
        decibels_full_scale(levels.left.rms),
        level_bar(levels.right.peak),
        decibels_full_scale(levels.right.peak),
        decibels_full_scale(levels.right.rms),
    );
    output.push_str("\n\nRTA 31.5 Hz – 16 kHz\n");
    output.push_str(&format_spectrum(frame.spectrum));
    output
}

fn format_spectrum(spectrum: Spectrum) -> String {
    const ROWS: usize = 10;
    const DB_PER_ROW: f32 = 6.0;
    let mut output = String::new();
    for row in 0..ROWS {
        let threshold = if row == 0 {
            0.0
        } else {
            -(row as f32) * DB_PER_ROW
        };
        output.push_str(&format!("{threshold:>3.0} |"));
        for amplitude in spectrum.bands {
            if decibels_full_scale(amplitude) >= threshold {
                output.push_str("█ ");
            } else {
                output.push_str("  ");
            }
        }
        output.push('\n');
    }
    output.push_str("    +");
    output.push_str(&"--".repeat(SPECTRUM_BAND_COUNT));
    output.push('\n');
    output.push_str("     32    63    125   250   500   1k    2k    4k    8k    16k");
    output
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
        Cli, Command, DeviceCommand, FilterCommand, FilterKind, ProfileCommand, SessionCli,
        SessionCommand, TerminalSize, equalizer_from_command, format_telemetry,
        format_terminal_telemetry, matching_profile_id, parse_terminal_size,
    };
    use clap::Parser as _;
    use tunic_dsp::{Equalizer, Filter};
    use tunic_engine::{ChannelLevels, ProfileId, Spectrum, StereoLevels, TelemetryFrame};

    #[test]
    fn formats_asymmetric_linear_levels_as_dbfs() {
        let frame = TelemetryFrame {
            levels: StereoLevels {
                left: ChannelLevels {
                    peak: 1.0,
                    rms: 0.5,
                },
                right: ChannelLevels {
                    peak: 0.25,
                    rms: 0.0,
                },
            },
            spectrum: Spectrum {
                bands: std::array::from_fn(|index| if index == 15 { 1.0 } else { 0.0 }),
            },
        };
        let output = format_telemetry(frame);

        assert!(output.contains("peak    0.0 dBFS  RMS   -6.0 dBFS"));
        assert!(output.contains("peak  -12.0 dBFS  RMS -120.0 dBFS"));
        assert!(output.contains("RTA 31.5 Hz – 16 kHz"));
        assert!(output.contains("  0 |                              █ "));

        let terminal = format_terminal_telemetry(
            frame,
            Some(TerminalSize {
                rows: 18,
                columns: 62,
            }),
        );
        assert_eq!(terminal.lines().count(), 18);
        assert!(terminal.lines().all(|line| line.chars().count() <= 62));
    }

    #[test]
    fn parses_terminal_dimensions_and_warns_when_the_viewport_is_too_small() {
        assert_eq!(
            parse_terminal_size(b"15 50\n"),
            Some(TerminalSize {
                rows: 15,
                columns: 50,
            })
        );
        assert_eq!(parse_terminal_size(b"15 50 extra\n"), None);

        let output = format_terminal_telemetry(
            TelemetryFrame::default(),
            Some(TerminalSize {
                rows: 15,
                columns: 50,
            }),
        );
        assert!(output.contains("Terminal too small: 50×15"));
        assert!(!output.contains("L ["));
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

        let parsed =
            Cli::try_parse_from(["tunic", "watch", "--data-directory", "/tmp/tunic-watch"])
                .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Watch {
                data_directory: Some(path),
            } if path == std::path::Path::new("/tmp/tunic-watch")
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
    fn parses_profile_management_commands() {
        let create = SessionCli::try_parse_from(
            shlex::split("profile create \"Studio Headphones\"").unwrap(),
        )
        .unwrap();
        assert!(matches!(
            create.command,
            SessionCommand::Profile {
                command: ProfileCommand::Create { name }
            } if name == "Studio Headphones"
        ));

        let rename = SessionCli::try_parse_from(
            shlex::split("profile rename headphones \"Desk Headphones\"").unwrap(),
        )
        .unwrap();
        assert!(matches!(
            rename.command,
            SessionCommand::Profile {
                command: ProfileCommand::Rename { profile, name }
            } if profile == "headphones" && name == "Desk Headphones"
        ));

        let assignment = SessionCli::try_parse_from(
            shlex::split("device set-profile headphones \"Studio Display Speakers\"").unwrap(),
        )
        .unwrap();
        assert!(matches!(
            assignment.command,
            SessionCommand::Device {
                command: DeviceCommand::SetProfile {
                    profile,
                    device: Some(device),
                }
            } if profile == "headphones" && device == "Studio Display Speakers"
        ));
    }

    #[test]
    fn profile_id_match_takes_precedence_over_a_name_match() {
        let alpha = ProfileId::new("alpha");
        let alpha_two = ProfileId::new("alpha-2");
        let profiles = [(&alpha_two, "alpha"), (&alpha, "Zulu")];

        assert_eq!(
            matching_profile_id(profiles.into_iter(), "alpha"),
            Some(&alpha)
        );
        assert_eq!(
            matching_profile_id(profiles.into_iter(), "ALPHA"),
            Some(&alpha_two)
        );
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
