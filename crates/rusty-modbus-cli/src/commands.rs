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
            let coils = client
                .read_coils(unit, args.address, args.quantity)
                .await?;
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
            let values: Vec<u16> = args
                .values
                .iter()
                .map(|v| v.parse::<u16>().unwrap_or(0))
                .collect();

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
            let values: Vec<bool> = args
                .values
                .iter()
                .map(|v| matches!(v.as_str(), "1" | "on" | "true" | "ON"))
                .collect();

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
