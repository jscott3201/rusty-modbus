//! Hand-rolled command parser for the Modbus shell.

/// Concise help lines shared by the line shell and dashboard command bar.
pub const HELP_LINES: &[&str] = &[
    "read <type> <addr> <qty>; types: coils, discrete-inputs",
    "read types: holding-registers, input-registers",
    "write <type> <addr> <values>; types: coil, coils, register, registers",
    "set unit-id <id> | discover units [range] | status | help | exit",
];

const COMMANDS: &[&str] = &[
    "discover", "exit", "help", "quit", "read", "set", "status", "write",
];
const DISCOVER_KEYS: &[&str] = &["units"];
const READ_TYPES: &[&str] = &[
    "coils",
    "discrete-inputs",
    "holding-registers",
    "input-registers",
];
const WRITE_TYPES: &[&str] = &["coil", "coils", "register", "registers"];
const SET_KEYS: &[&str] = &["unit-id"];

/// Parsed shell command.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellCommand {
    /// Read coils (FC 0x01).
    ReadCoils { address: u16, quantity: u16 },
    /// Read discrete inputs (FC 0x02).
    ReadDiscreteInputs { address: u16, quantity: u16 },
    /// Read holding registers (FC 0x03).
    ReadHoldingRegisters { address: u16, quantity: u16 },
    /// Read input registers (FC 0x04).
    ReadInputRegisters { address: u16, quantity: u16 },
    /// Write single coil (FC 0x05).
    WriteCoil { address: u16, value: bool },
    /// Write multiple coils (FC 0x0F).
    WriteCoils { address: u16, values: Vec<bool> },
    /// Write single register (FC 0x06).
    WriteRegister { address: u16, value: u16 },
    /// Write multiple registers (FC 0x10).
    WriteRegisters { address: u16, values: Vec<u16> },
    /// Discover responding unit IDs on the current endpoint.
    DiscoverUnits { unit_id_range: String },
    /// Change the active unit ID.
    SetUnitId(u8),
    /// Show connection status.
    Status,
    /// Print available commands.
    Help,
    /// Exit the shell.
    Exit,
    /// Empty input line (no-op).
    Empty,
}

/// Parse error.
#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a single shell input line into a command.
///
/// # Errors
///
/// Returns `ParseError` for unrecognized commands or invalid arguments.
pub fn parse_command(line: &str) -> Result<ShellCommand, ParseError> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(ShellCommand::Empty);
    }

    match tokens[0] {
        "help" => Ok(ShellCommand::Help),
        "status" => Ok(ShellCommand::Status),
        "exit" | "quit" => Ok(ShellCommand::Exit),
        "discover" => parse_discover(&tokens[1..]),
        "set" => parse_set(&tokens[1..]),
        "read" => parse_read(&tokens[1..]),
        "write" => parse_write(&tokens[1..]),
        other => Err(ParseError(format!(
            "unknown command: '{other}'. Type 'help' for available commands"
        ))),
    }
}

/// Return a useful command completion for a partial shell input line.
#[must_use]
pub fn complete_command(line: &str) -> Option<String> {
    let trailing_space = line.chars().last().is_some_and(char::is_whitespace);
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let (prefix, candidates, token_index) = match tokens.as_slice() {
        [] => ("", COMMANDS, 0),
        [partial] if !trailing_space => (*partial, COMMANDS, 0),
        ["read"] if trailing_space => ("", READ_TYPES, 1),
        ["read", partial] if !trailing_space => (*partial, READ_TYPES, 1),
        ["write"] if trailing_space => ("", WRITE_TYPES, 1),
        ["write", partial] if !trailing_space => (*partial, WRITE_TYPES, 1),
        ["set"] if trailing_space => ("", SET_KEYS, 1),
        ["set", partial] if !trailing_space => (*partial, SET_KEYS, 1),
        ["discover"] if trailing_space => ("", DISCOVER_KEYS, 1),
        ["discover", partial] if !trailing_space => (*partial, DISCOVER_KEYS, 1),
        _ => return None,
    };
    let completion = complete_token(prefix, candidates)?;
    Some(replace_token(&tokens, token_index, &completion))
}

fn complete_token(prefix: &str, candidates: &[&str]) -> Option<String> {
    let matches = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.starts_with(prefix))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => None,
        [completion] => Some((*completion).to_string()),
        matches => {
            let completion = common_prefix(matches);
            (completion.len() > prefix.len()).then_some(completion)
        }
    }
}

fn common_prefix(matches: &[&str]) -> String {
    let mut prefix = matches[0].to_string();
    for candidate in &matches[1..] {
        while !candidate.starts_with(&prefix) {
            prefix.pop();
        }
    }
    prefix
}

fn replace_token(tokens: &[&str], index: usize, completion: &str) -> String {
    let mut completed = tokens.to_vec();
    if index == completed.len() {
        completed.push(completion);
    } else {
        completed[index] = completion;
    }
    completed.join(" ")
}

fn parse_discover(tokens: &[&str]) -> Result<ShellCommand, ParseError> {
    match tokens {
        [] => Ok(ShellCommand::DiscoverUnits {
            unit_id_range: "1-247".to_string(),
        }),
        ["units"] => Ok(ShellCommand::DiscoverUnits {
            unit_id_range: "1-247".to_string(),
        }),
        ["units", range] => Ok(ShellCommand::DiscoverUnits {
            unit_id_range: (*range).to_string(),
        }),
        _ => Err(ParseError("usage: discover units [unit-id-range]".into())),
    }
}

fn parse_set(tokens: &[&str]) -> Result<ShellCommand, ParseError> {
    if tokens.first() != Some(&"unit-id") || tokens.len() != 2 {
        return Err(ParseError("usage: set unit-id <id>".into()));
    }
    let id = parse_u8(tokens[1])?;
    Ok(ShellCommand::SetUnitId(id))
}

fn parse_read(tokens: &[&str]) -> Result<ShellCommand, ParseError> {
    if tokens.len() != 3 {
        return Err(ParseError("usage: read <type> <address> <quantity>".into()));
    }
    let address = parse_u16(tokens[1])?;
    let quantity = parse_u16(tokens[2])?;
    match tokens[0] {
        "coils" => Ok(ShellCommand::ReadCoils { address, quantity }),
        "discrete-inputs" => Ok(ShellCommand::ReadDiscreteInputs { address, quantity }),
        "holding-registers" => Ok(ShellCommand::ReadHoldingRegisters { address, quantity }),
        "input-registers" => Ok(ShellCommand::ReadInputRegisters { address, quantity }),
        other => Err(ParseError(format!(
            "unknown register type: '{other}'. Use: coils, discrete-inputs, holding-registers, input-registers"
        ))),
    }
}

fn parse_write(tokens: &[&str]) -> Result<ShellCommand, ParseError> {
    if tokens.len() < 3 {
        return Err(ParseError(
            "usage: write <type> <address> <value(s)>".into(),
        ));
    }
    let address = parse_u16(tokens[1])?;
    match tokens[0] {
        "coil" => {
            if tokens.len() != 3 {
                return Err(ParseError("usage: write coil <address> <on|off>".into()));
            }
            let value = parse_bool(tokens[2])?;
            Ok(ShellCommand::WriteCoil { address, value })
        }
        "coils" => {
            let values: Result<Vec<bool>, _> = tokens[2..].iter().map(|t| parse_bool(t)).collect();
            Ok(ShellCommand::WriteCoils {
                address,
                values: values?,
            })
        }
        "register" => {
            if tokens.len() != 3 {
                return Err(ParseError("usage: write register <address> <value>".into()));
            }
            let value = parse_u16(tokens[2])?;
            Ok(ShellCommand::WriteRegister { address, value })
        }
        "registers" => {
            let values: Result<Vec<u16>, _> = tokens[2..].iter().map(|t| parse_u16(t)).collect();
            Ok(ShellCommand::WriteRegisters {
                address,
                values: values?,
            })
        }
        other => Err(ParseError(format!(
            "unknown register type: '{other}'. Use: coil, coils, register, registers"
        ))),
    }
}

fn parse_u16(s: &str) -> Result<u16, ParseError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
            .map_err(|e| ParseError(format!("invalid hex value '{s}': {e}")))
    } else {
        s.parse()
            .map_err(|e| ParseError(format!("invalid number '{s}': {e}")))
    }
}

fn parse_u8(s: &str) -> Result<u8, ParseError> {
    s.parse()
        .map_err(|e| ParseError(format!("invalid number '{s}': {e}")))
}

fn parse_bool(s: &str) -> Result<bool, ParseError> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        other => Err(ParseError(format!(
            "invalid coil value '{other}'. Use: on/off, true/false, 1/0"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_read_holding_registers() {
        let cmd = parse_command("read holding-registers 100 10").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::ReadHoldingRegisters {
                address: 100,
                quantity: 10
            }
        );
    }

    #[test]
    fn parse_read_coils() {
        let cmd = parse_command("read coils 0 8").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::ReadCoils {
                address: 0,
                quantity: 8
            }
        );
    }

    #[test]
    fn parse_read_discrete_inputs() {
        let cmd = parse_command("read discrete-inputs 5 16").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::ReadDiscreteInputs {
                address: 5,
                quantity: 16
            }
        );
    }

    #[test]
    fn parse_read_input_registers() {
        let cmd = parse_command("read input-registers 0 1").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::ReadInputRegisters {
                address: 0,
                quantity: 1
            }
        );
    }

    #[test]
    fn parse_write_register_decimal() {
        let cmd = parse_command("write register 5 1234").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteRegister {
                address: 5,
                value: 1234
            }
        );
    }

    #[test]
    fn parse_write_register_hex() {
        let cmd = parse_command("write register 5 0xBEEF").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteRegister {
                address: 5,
                value: 0xBEEF
            }
        );
    }

    #[test]
    fn parse_write_coil_on_off() {
        let cmd = parse_command("write coil 3 on").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteCoil {
                address: 3,
                value: true
            }
        );
        let cmd = parse_command("write coil 3 OFF").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteCoil {
                address: 3,
                value: false
            }
        );
    }

    #[test]
    fn parse_write_registers_multiple() {
        let cmd = parse_command("write registers 0 100 200 0xFF").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteRegisters {
                address: 0,
                values: vec![100, 200, 255]
            }
        );
    }

    #[test]
    fn parse_write_coils_multiple() {
        let cmd = parse_command("write coils 0 on off true 0").unwrap();
        assert_eq!(
            cmd,
            ShellCommand::WriteCoils {
                address: 0,
                values: vec![true, false, true, false]
            }
        );
    }

    #[test]
    fn parse_set_unit_id() {
        let cmd = parse_command("set unit-id 5").unwrap();
        assert_eq!(cmd, ShellCommand::SetUnitId(5));
    }

    #[test]
    fn parse_discover_units() {
        assert_eq!(
            parse_command("discover").unwrap(),
            ShellCommand::DiscoverUnits {
                unit_id_range: "1-247".to_string()
            }
        );
        assert_eq!(
            parse_command("discover units").unwrap(),
            ShellCommand::DiscoverUnits {
                unit_id_range: "1-247".to_string()
            }
        );
        assert_eq!(
            parse_command("discover units 1-10").unwrap(),
            ShellCommand::DiscoverUnits {
                unit_id_range: "1-10".to_string()
            }
        );
    }

    #[test]
    fn parse_builtins() {
        assert_eq!(parse_command("help").unwrap(), ShellCommand::Help);
        assert_eq!(parse_command("status").unwrap(), ShellCommand::Status);
        assert_eq!(parse_command("exit").unwrap(), ShellCommand::Exit);
        assert_eq!(parse_command("quit").unwrap(), ShellCommand::Exit);
    }

    #[test]
    fn parse_empty_line() {
        assert_eq!(parse_command("").unwrap(), ShellCommand::Empty);
        assert_eq!(parse_command("   ").unwrap(), ShellCommand::Empty);
    }

    #[test]
    fn parse_unknown_command() {
        assert!(parse_command("foo bar").is_err());
    }

    #[test]
    fn parse_unknown_register_type() {
        assert!(parse_command("read foobar 0 1").is_err());
        assert!(parse_command("write foobar 0 1").is_err());
    }

    #[test]
    fn parse_invalid_discover_command() {
        assert!(parse_command("discover hosts 1-10").is_err());
        assert!(parse_command("discover units 1 2").is_err());
    }

    #[test]
    fn complete_top_level_commands() {
        assert_eq!(complete_command("sta").as_deref(), Some("status"));
        assert_eq!(complete_command("he").as_deref(), Some("help"));
        assert_eq!(complete_command("w").as_deref(), Some("write"));
    }

    #[test]
    fn complete_read_and_write_types() {
        assert_eq!(
            complete_command("read h").as_deref(),
            Some("read holding-registers")
        );
        assert_eq!(
            complete_command("read ").as_deref(),
            None,
            "empty read type is ambiguous"
        );
        assert_eq!(
            complete_command("write regi").as_deref(),
            Some("write register")
        );
        assert_eq!(complete_command("write co").as_deref(), Some("write coil"));
    }

    #[test]
    fn complete_set_unit_id_key() {
        assert_eq!(complete_command("set u").as_deref(), Some("set unit-id"));
        assert_eq!(complete_command("set ").as_deref(), Some("set unit-id"));
    }

    #[test]
    fn complete_discover_units_key() {
        assert_eq!(
            complete_command("discover u").as_deref(),
            Some("discover units")
        );
        assert_eq!(
            complete_command("discover ").as_deref(),
            Some("discover units")
        );
    }

    #[test]
    fn complete_ignores_ambiguous_or_argument_positions() {
        assert_eq!(complete_command("s").as_deref(), None);
        assert_eq!(complete_command("write ").as_deref(), None);
        assert_eq!(complete_command("read coils 0").as_deref(), None);
    }

    #[test]
    fn help_lines_cover_supported_commands() {
        let help = HELP_LINES.join("\n");
        for command in [
            "coils",
            "discrete-inputs",
            "holding-registers",
            "input-registers",
            "registers",
            "discover units",
            "set unit-id",
            "status",
            "exit",
        ] {
            assert!(help.contains(command), "missing help for {command}");
        }
        for line in HELP_LINES {
            assert!(line.len() <= 70, "help line too wide for dashboard: {line}");
        }
    }
}
