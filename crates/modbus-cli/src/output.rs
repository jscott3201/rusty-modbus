//! Output formatting — human-readable and JSON.

use serde::Serialize;

/// Output format selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable table format.
    Human,
    /// JSON format for scripting.
    Json,
}

/// Print register values.
pub fn print_registers(address: u16, values: &[u16], format: OutputFormat) {
    match format {
        OutputFormat::Human => {
            for (i, &v) in values.iter().enumerate() {
                let addr = address + u16::try_from(i).unwrap_or(u16::MAX);
                println!("Register {addr:>5}: {v:#06X} ({v})");
            }
        }
        OutputFormat::Json => {
            let out = RegisterOutput {
                function: "read_registers",
                address,
                values,
            };
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
    }
}

/// Print coil/discrete input values.
pub fn print_coils(address: u16, values: &[bool], kind: &str, format: OutputFormat) {
    match format {
        OutputFormat::Human => {
            for (i, &v) in values.iter().enumerate() {
                let addr = address + u16::try_from(i).unwrap_or(u16::MAX);
                let state = if v { "ON" } else { "OFF" };
                println!("{kind} {addr:>5}: {state}");
            }
        }
        OutputFormat::Json => {
            let out = CoilOutput {
                function: kind,
                address,
                values,
            };
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
    }
}

/// Print a success message for write operations.
pub fn print_write_ok(format: OutputFormat) {
    match format {
        OutputFormat::Human => println!("OK"),
        OutputFormat::Json => println!(r#"{{"status": "ok"}}"#),
    }
}

#[derive(Serialize)]
struct RegisterOutput<'a> {
    function: &'a str,
    address: u16,
    values: &'a [u16],
}

#[derive(Serialize)]
struct CoilOutput<'a> {
    function: &'a str,
    address: u16,
    values: &'a [bool],
}
