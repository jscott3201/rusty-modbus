//! Interactive Modbus shell — REPL with persistent connection.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_types::UnitId;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::output::{self, OutputFormat};
use crate::shell_parser::{self, ShellCommand};

/// Shell session configuration (from CLI args).
pub struct ShellConfig {
    /// Target address.
    pub addr: SocketAddr,
    /// Initial unit ID.
    pub unit_id: u8,
    /// Request timeout in seconds.
    pub timeout: u64,
    /// Output format.
    pub format: OutputFormat,
}

/// Run the interactive shell.
///
/// # Errors
///
/// Returns an error if the initial connection fails or readline encounters
/// an unrecoverable error.
pub async fn run(config: ShellConfig) -> Result<(), Box<dyn std::error::Error>> {
    let mut unit_id = UnitId(config.unit_id);

    println!("Connecting to {}...", config.addr);
    let mut client = connect(&config, unit_id).await?;
    println!("Connected. Type 'help' for commands, 'exit' to quit.\n");

    let mut rl = DefaultEditor::new()?;
    let history_path = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".modbus_history"));
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let prompt = format!("modbus[{}]> ", unit_id.0);
        let line = match rl.readline(&prompt) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(trimmed);

        let cmd = match shell_parser::parse_command(trimmed) {
            Ok(cmd) => cmd,
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
        };

        match cmd {
            ShellCommand::Exit => break,
            ShellCommand::Empty => continue,
            ShellCommand::Help => print_help(),
            ShellCommand::Status => {
                println!("Host:      {}", config.addr);
                println!("Unit ID:   {}", unit_id.0);
                println!("Connected: {}", client.is_connected());
            }
            ShellCommand::SetUnitId(id) => {
                unit_id = UnitId(id);
                println!("Unit ID set to {id}");
            }
            _ => {
                match execute_command(&client, unit_id, &cmd, config.format).await {
                    Ok(()) => {}
                    Err(ClientError::Transport(_)) | Err(ClientError::NotConnected) => {
                        eprintln!("Connection lost. Reconnecting...");
                        match connect(&config, unit_id).await {
                            Ok(new_client) => {
                                client = new_client;
                                println!("Reconnected.");
                                if let Err(e) =
                                    execute_command(&client, unit_id, &cmd, config.format).await
                                {
                                    eprintln!("Error: {e}");
                                }
                            }
                            Err(e) => eprintln!("Reconnect failed: {e}"),
                        }
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
        }
    }

    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }
    println!("Goodbye.");
    Ok(())
}

async fn connect(
    config: &ShellConfig,
    unit_id: UnitId,
) -> Result<ModbusClient, Box<dyn std::error::Error>> {
    let client_config = ClientConfig {
        unit_id,
        timeout: Duration::from_secs(config.timeout),
        ..ClientConfig::default()
    };
    Ok(ModbusClient::connect(config.addr, client_config).await?)
}

async fn execute_command(
    client: &ModbusClient,
    unit_id: UnitId,
    cmd: &ShellCommand,
    fmt: OutputFormat,
) -> Result<(), ClientError> {
    match cmd {
        ShellCommand::ReadCoils { address, quantity } => {
            let coils = client.read_coils(unit_id, *address, *quantity).await?;
            output::print_coils(*address, &coils, "Coil", fmt);
        }
        ShellCommand::ReadDiscreteInputs { address, quantity } => {
            let inputs = client
                .read_discrete_inputs(unit_id, *address, *quantity)
                .await?;
            output::print_coils(*address, &inputs, "Discrete", fmt);
        }
        ShellCommand::ReadHoldingRegisters { address, quantity } => {
            let regs = client
                .read_holding_registers(unit_id, *address, *quantity)
                .await?;
            output::print_registers(*address, &regs, fmt);
        }
        ShellCommand::ReadInputRegisters { address, quantity } => {
            let regs = client
                .read_input_registers(unit_id, *address, *quantity)
                .await?;
            output::print_registers(*address, &regs, fmt);
        }
        ShellCommand::WriteCoil { address, value } => {
            client.write_single_coil(unit_id, *address, *value).await?;
            output::print_write_ok(fmt);
        }
        ShellCommand::WriteCoils { address, values } => {
            client
                .write_multiple_coils(unit_id, *address, values)
                .await?;
            output::print_write_ok(fmt);
        }
        ShellCommand::WriteRegister { address, value } => {
            client
                .write_single_register(unit_id, *address, *value)
                .await?;
            output::print_write_ok(fmt);
        }
        ShellCommand::WriteRegisters { address, values } => {
            client
                .write_multiple_registers(unit_id, *address, values)
                .await?;
            output::print_write_ok(fmt);
        }
        ShellCommand::Help | ShellCommand::Status | ShellCommand::Exit | ShellCommand::Empty | ShellCommand::SetUnitId(_) => {
            // Handled in run() directly.
        }
    }
    Ok(())
}

fn print_help() {
    println!("Commands:");
    println!("  read coils <address> <quantity>");
    println!("  read discrete-inputs <address> <quantity>");
    println!("  read holding-registers <address> <quantity>");
    println!("  read input-registers <address> <quantity>");
    println!("  write coil <address> <on|off>");
    println!("  write coils <address> <value> [<value>...]");
    println!("  write register <address> <value>");
    println!("  write registers <address> <value> [<value>...]");
    println!("  set unit-id <id>");
    println!("  status");
    println!("  help");
    println!("  exit");
    println!();
    println!("Values: registers accept decimal or 0x hex. Coils accept on/off, true/false, 1/0.");
}
