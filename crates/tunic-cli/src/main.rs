//! Tunic's headless reference client.

mod capture;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
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
        SessionCommand::Help => print_session_help()?,
        SessionCommand::Quit => return Ok(true),
    }
    Ok(false)
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
    use super::{DeviceCommand, SessionCli, SessionCommand};
    use clap::Parser as _;

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
}
