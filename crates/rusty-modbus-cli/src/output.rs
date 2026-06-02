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
                let addr = display_address(address, i);
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
                let addr = display_address(address, i);
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

fn display_address(address: u16, offset: usize) -> u32 {
    u32::from(address).saturating_add(u32::try_from(offset).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::display_address;

    #[test]
    fn display_address_preserves_normal_range() {
        assert_eq!(display_address(10, 3), 13);
    }

    #[test]
    fn display_address_handles_u16_boundary_without_wrapping() {
        assert_eq!(display_address(u16::MAX, 0), 65_535);
        assert_eq!(display_address(u16::MAX, 1), 65_536);
    }

    #[test]
    fn display_address_saturates_extreme_offsets() {
        assert_eq!(display_address(u16::MAX, usize::MAX), u32::MAX);
    }
}
