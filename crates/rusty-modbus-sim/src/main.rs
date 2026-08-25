//! Process entry point for the YAML-configured Modbus/TCP simulator.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rusty_modbus_sim::ModbusSimulator;

const HELP: &str = concat!(
    "Usage: rusty-modbus-sim <CONFIG>\n",
    "\n",
    "Run a Modbus/TCP simulator from one validated YAML file.\n",
    "\n",
    "Options:\n",
    "  -h, --help  Print help\n",
);
const STOPPED_LINE: &str = "RUSTY_MODBUS_SIM_STOPPED";

#[derive(Debug)]
enum Arguments {
    Help,
    Run(PathBuf),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(Arguments::Help) => match write_stdout(HELP) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => report_runtime_error(&error),
        },
        Ok(Arguments::Run(path)) => match run(&path).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => report_runtime_error(&error),
        },
        Err(error) => {
            eprintln!("rusty-modbus-sim: {error}");
            eprintln!("Try 'rusty-modbus-sim --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn parse_args(args: impl Iterator<Item = OsString>) -> Result<Arguments, String> {
    let mut args = args;
    let Some(argument) = args.next() else {
        return Err(String::from("missing CONFIG"));
    };
    if let Some(extra) = args.next() {
        return Err(format!(
            "unexpected argument {:?}; expected one CONFIG path",
            extra.to_string_lossy()
        ));
    }

    if argument == "-h" || argument == "--help" {
        return Ok(Arguments::Help);
    }
    if argument.to_string_lossy().starts_with('-') {
        return Err(format!("unknown option {:?}", argument.to_string_lossy()));
    }

    Ok(Arguments::Run(PathBuf::from(argument)))
}

async fn run(path: &Path) -> Result<(), String> {
    let yaml = fs::read_to_string(path)
        .map_err(|error| format!("failed to read config {}: {error}", path.display()))?;
    let mut simulator = ModbusSimulator::from_yaml(&yaml)
        .map_err(|error| format!("failed to load config {}: {error}", path.display()))?;
    let unit_id = simulator.unit_id().0;
    let signals = ShutdownSignals::install()?;
    let address = simulator
        .start()
        .await
        .map_err(|error| format!("failed to start simulator: {error}"))?;

    let readiness = format!("RUSTY_MODBUS_SIM_READY address={address} unit_id={unit_id}\n");
    if let Err(error) = write_stdout(&readiness) {
        simulator.stop().await;
        return Err(error);
    }

    if let Err(error) = signals.wait().await {
        simulator.stop().await;
        return Err(error);
    }

    simulator.stop().await;
    write_stdout(&format!("{STOPPED_LINE}\n"))
}

fn write_stdout(output: &str) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| format!("failed to write stdout: {error}"))
}

fn report_runtime_error(error: &str) -> ExitCode {
    eprintln!("rusty-modbus-sim: {error}");
    ExitCode::FAILURE
}

#[cfg(unix)]
struct ShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignals {
    fn install() -> Result<Self, String> {
        let interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .map_err(|error| format!("failed to install SIGINT handler: {error}"))?;
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|error| format!("failed to install SIGTERM handler: {error}"))?;
        Ok(Self {
            interrupt,
            terminate,
        })
    }

    async fn wait(mut self) -> Result<(), String> {
        tokio::select! {
            signal = self.interrupt.recv() => signal
                .ok_or_else(|| String::from("SIGINT handler closed before receiving a signal")),
            signal = self.terminate.recv() => signal
                .ok_or_else(|| String::from("SIGTERM handler closed before receiving a signal")),
        }
    }
}

#[cfg(not(unix))]
struct ShutdownSignals;

#[cfg(not(unix))]
impl ShutdownSignals {
    fn install() -> Result<Self, String> {
        Ok(Self)
    }

    async fn wait(self) -> Result<(), String> {
        tokio::signal::ctrl_c()
            .await
            .map_err(|error| format!("failed to wait for Ctrl-C: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Arguments, HELP, parse_args};
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn help_text_is_stable() {
        assert_eq!(
            HELP,
            "Usage: rusty-modbus-sim <CONFIG>\n\n\
             Run a Modbus/TCP simulator from one validated YAML file.\n\n\
             Options:\n  -h, --help  Print help\n"
        );
    }

    #[test]
    fn parser_accepts_one_path_or_help() {
        assert!(matches!(
            parse_args([OsString::from("device.yaml")].into_iter()).unwrap(),
            Arguments::Run(path) if path == Path::new("device.yaml")
        ));
        assert!(matches!(
            parse_args([OsString::from("--help")].into_iter()).unwrap(),
            Arguments::Help
        ));
    }

    #[test]
    fn parser_rejects_missing_extra_and_unknown_arguments() {
        assert_eq!(
            parse_args(std::iter::empty()).unwrap_err(),
            "missing CONFIG"
        );
        assert!(
            parse_args([OsString::from("one.yaml"), OsString::from("two.yaml")].into_iter())
                .unwrap_err()
                .contains("unexpected argument")
        );
        assert!(
            parse_args([OsString::from("--version")].into_iter())
                .unwrap_err()
                .contains("unknown option")
        );
    }
}
