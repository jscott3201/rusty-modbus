//! CLI command implementations — read and write.

use clap::Args;
use rusty_modbus_client::{ClientError, ModbusClient};
use rusty_modbus_types::UnitId;

use crate::output::{self, OutputFormat};

/// Arguments for the `read` subcommand.
#[derive(Args, Debug)]
pub struct ReadArgs {
    /// Register type: `hr` (holding), `ir` (input), `co` (coils), `di` (discrete).
    pub register_type: String,

    /// Starting address.
    pub address: u16,

    /// Quantity to read.
    pub quantity: u16,
}

/// Arguments for the `write` subcommand.
#[derive(Args, Debug)]
pub struct WriteArgs {
    /// Register type: `hr` (holding register), `co` (coil).
    pub register_type: String,

    /// Starting address.
    pub address: u16,

    /// Values to write. For coils: `1`/`0` or `on`/`off`.
    pub values: Vec<String>,
}

/// Handle the `read` command.
pub async fn handle_read(
    client: &ModbusClient,
    unit: UnitId,
    args: &ReadArgs,
    fmt: OutputFormat,
) -> Result<(), ClientError> {
    match args.register_type.as_str() {
        "hr" | "holding" => {
            let regs = client
                .read_holding_registers(unit, args.address, args.quantity)
                .await?;
            output::print_registers(args.address, &regs, fmt);
        }
        "ir" | "input" => {
            let regs = client
                .read_input_registers(unit, args.address, args.quantity)
                .await?;
            output::print_registers(args.address, &regs, fmt);
        }
        "co" | "coils" => {
            let coils = client.read_coils(unit, args.address, args.quantity).await?;
            output::print_coils(args.address, &coils, "Coil", fmt);
        }
        "di" | "discrete" => {
            let inputs = client
                .read_discrete_inputs(unit, args.address, args.quantity)
                .await?;
            output::print_coils(args.address, &inputs, "Discrete", fmt);
        }
        other => {
            eprintln!("Unknown register type: '{other}'. Use: hr, ir, co, di");
            return Err(ClientError::NotConnected); // reuse as generic error
        }
    }
    Ok(())
}

/// Handle the `write` command.
pub async fn handle_write(
    client: &ModbusClient,
    unit: UnitId,
    args: &WriteArgs,
    fmt: OutputFormat,
) -> Result<(), ClientError> {
    match args.register_type.as_str() {
        "hr" | "holding" => {
            let values = parse_register_values(&args.values)?;

            if values.len() == 1 {
                client
                    .write_single_register(unit, args.address, values[0])
                    .await?;
            } else {
                client
                    .write_multiple_registers(unit, args.address, &values)
                    .await?;
            }
            output::print_write_ok(fmt);
        }
        "co" | "coil" | "coils" => {
            let values = parse_coil_values(&args.values)?;

            if values.len() == 1 {
                client
                    .write_single_coil(unit, args.address, values[0])
                    .await?;
            } else {
                client
                    .write_multiple_coils(unit, args.address, &values)
                    .await?;
            }
            output::print_write_ok(fmt);
        }
        other => {
            eprintln!("Unknown register type: '{other}'. Use: hr, co");
            return Err(ClientError::NotConnected);
        }
    }
    Ok(())
}

fn parse_register_values(values: &[String]) -> Result<Vec<u16>, ClientError> {
    parse_non_empty(values)?;
    values
        .iter()
        .map(|value| parse_u16(value).map_err(cli_parse_error))
        .collect()
}

fn parse_coil_values(values: &[String]) -> Result<Vec<bool>, ClientError> {
    parse_non_empty(values)?;
    values
        .iter()
        .map(|value| parse_bool(value).map_err(cli_parse_error))
        .collect()
}

fn parse_non_empty(values: &[String]) -> Result<(), ClientError> {
    if values.is_empty() {
        return Err(cli_parse_error(
            "at least one value is required for write commands",
        ));
    }
    Ok(())
}

fn parse_u16(value: &str) -> Result<u16, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16)
            .map_err(|error| format!("invalid register value '{value}': {error}"))
    } else {
        value
            .parse()
            .map_err(|error| format!("invalid register value '{value}': {error}"))
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        _ => Err(format!(
            "invalid coil value '{value}'. Use: on/off, true/false, 1/0"
        )),
    }
}

fn cli_parse_error(message: impl AsRef<str>) -> ClientError {
    eprintln!("Error: {}", message.as_ref());
    ClientError::NotConnected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parse_register_values_accepts_decimal_and_hex() {
        assert_eq!(
            parse_register_values(&values(&["42", "0xBEEF", "0X10"])).unwrap(),
            vec![42, 0xBEEF, 0x10]
        );
    }

    #[test]
    fn parse_register_values_rejects_invalid_input() {
        assert!(parse_register_values(&values(&["nope"])).is_err());
    }

    #[test]
    fn parse_coil_values_accepts_common_forms_case_insensitively() {
        assert_eq!(
            parse_coil_values(&values(&["on", "OFF", "true", "False", "1", "0"])).unwrap(),
            vec![true, false, true, false, true, false]
        );
    }

    #[test]
    fn parse_coil_values_rejects_unknown_input() {
        assert!(parse_coil_values(&values(&["enabled"])).is_err());
    }

    #[test]
    fn parse_write_values_require_at_least_one_value() {
        assert!(parse_register_values(&[]).is_err());
        assert!(parse_coil_values(&[]).is_err());
    }
}
