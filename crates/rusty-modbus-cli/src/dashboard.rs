//! Ratatui dashboard for interactive Modbus diagnostics.

use std::collections::VecDeque;
use std::error::Error;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_types::UnitId;

use crate::shell_parser::{self, ShellCommand};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_COMMAND_LOG: usize = 8;
const MAX_COMMAND_HISTORY: usize = 32;

mod render;

mod palette {
    use ratatui::style::Color;

    pub const BACKGROUND: Color = Color::Rgb(8, 20, 31);
    pub const PANEL: Color = Color::Rgb(13, 35, 52);
    pub const STEEL: Color = Color::Rgb(48, 103, 145);
    pub const CYAN: Color = Color::Rgb(91, 192, 222);
    pub const TEXT: Color = Color::Rgb(219, 232, 240);
    pub const MUTED: Color = Color::Rgb(126, 153, 169);
    pub const AMBER: Color = Color::Rgb(226, 170, 62);
    pub const GREEN: Color = Color::Rgb(70, 180, 130);
    pub const RED: Color = Color::Rgb(219, 96, 96);
}

/// Arguments for the `dashboard` subcommand.
#[derive(clap::Args, Debug)]
pub struct DashboardArgs {
    /// Initial data area to display.
    #[arg(long, value_enum, default_value = "holding-registers")]
    pub target: DashboardTarget,

    /// Initial starting address.
    #[arg(long, default_value_t = 0)]
    pub address: u16,

    /// Initial quantity to read.
    #[arg(long, short = 'q', default_value_t = 16)]
    pub quantity: u16,

    /// Auto-refresh interval in seconds. Use 0 for manual refresh only.
    #[arg(long, default_value_t = 2)]
    pub refresh_secs: u64,
}

/// Runtime configuration for the dashboard.
pub struct DashboardConfig {
    pub addr: SocketAddr,
    pub unit_id: u8,
    pub timeout: u64,
    pub address: u16,
    pub quantity: u16,
    pub target: DashboardTarget,
    pub refresh_interval: Duration,
}

/// Data area displayed by the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DashboardTarget {
    /// Holding registers, function code 0x03.
    HoldingRegisters,
    /// Input registers, function code 0x04.
    InputRegisters,
    /// Coils, function code 0x01.
    Coils,
    /// Discrete inputs, function code 0x02.
    DiscreteInputs,
}

impl DashboardTarget {
    const ALL: [Self; 4] = [
        Self::HoldingRegisters,
        Self::InputRegisters,
        Self::Coils,
        Self::DiscreteInputs,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::HoldingRegisters => "Holding Registers",
            Self::InputRegisters => "Input Registers",
            Self::Coils => "Coils",
            Self::DiscreteInputs => "Discrete Inputs",
        }
    }

    const fn short_label(self) -> &'static str {
        match self {
            Self::HoldingRegisters => "HR",
            Self::InputRegisters => "IR",
            Self::Coils => "CO",
            Self::DiscreteInputs => "DI",
        }
    }

    const fn function_code(self) -> &'static str {
        match self {
            Self::HoldingRegisters => "0x03",
            Self::InputRegisters => "0x04",
            Self::Coils => "0x01",
            Self::DiscreteInputs => "0x02",
        }
    }

    const fn max_quantity(self) -> u16 {
        match self {
            Self::HoldingRegisters | Self::InputRegisters => 125,
            Self::Coils | Self::DiscreteInputs => 2000,
        }
    }

    fn tab_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(0)
    }

    fn next(self) -> Self {
        let index = (self.tab_index() + 1) % Self::ALL.len();
        Self::ALL[index]
    }

    fn previous(self) -> Self {
        let index = self
            .tab_index()
            .checked_sub(1)
            .unwrap_or(Self::ALL.len() - 1);
        Self::ALL[index]
    }
}

/// Run the dashboard.
///
/// # Errors
///
/// Returns an error when the initial connection fails, terminal setup fails, or the terminal event
/// loop cannot read input.
pub async fn run(config: DashboardConfig) -> Result<(), Box<dyn Error>> {
    let client_config = ClientConfig {
        unit_id: UnitId(config.unit_id),
        timeout: Duration::from_secs(config.timeout),
        ..ClientConfig::default()
    };
    let client = ModbusClient::connect(config.addr, client_config).await?;

    let mut app = DashboardApp::new(config, client);
    let mut terminal = TerminalSession::enter()?;
    run_loop(terminal.terminal_mut(), &mut app).await
}

async fn run_loop(
    terminal: &mut DefaultTerminal,
    app: &mut DashboardApp,
) -> Result<(), Box<dyn Error>> {
    while !app.should_quit {
        app.update_refresh_age();
        terminal.draw(|frame| render::render_dashboard(frame, &app.view))?;

        if app.needs_initial_refresh() || app.should_auto_refresh() {
            let _ = app.refresh().await;
            continue;
        }

        let Some(Event::Key(key)) = read_event(EVENT_POLL_INTERVAL)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if app.handle_key(key).await {
            let _ = app.refresh().await;
        }
    }
    Ok(())
}

fn read_event(timeout: Duration) -> Result<Option<Event>, Box<dyn Error>> {
    if event::poll(timeout)? {
        Ok(Some(event::read()?))
    } else {
        Ok(None)
    }
}

struct TerminalSession {
    terminal: DefaultTerminal,
}

impl TerminalSession {
    fn enter() -> std::io::Result<Self> {
        Ok(Self {
            terminal: ratatui::try_init()?,
        })
    }

    fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = ratatui::try_restore();
    }
}

struct DashboardApp {
    client: ModbusClient,
    view: DashboardView,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    command_history: VecDeque<String>,
    command_history_cursor: Option<usize>,
    should_quit: bool,
}

impl DashboardApp {
    fn new(config: DashboardConfig, client: ModbusClient) -> Self {
        let mut view = DashboardView {
            endpoint: config.addr.to_string(),
            unit_id: config.unit_id,
            timeout_secs: config.timeout,
            target: config.target,
            address: config.address,
            quantity: config.quantity,
            data: DashboardData::Empty,
            status: DashboardStatus::Connected,
            refresh_count: 0,
            refresh_age: "not refreshed".to_string(),
            message: "Ready".to_string(),
            command_mode: CommandMode::Idle,
            command_input: String::new(),
            command_log: VecDeque::from([CommandLogEntry::info(
                "Press ':' to run read/write/status/help commands.",
            )]),
        };
        view.clamp_quantity();

        Self {
            client,
            view,
            refresh_interval: config.refresh_interval,
            last_refresh: None,
            command_history: VecDeque::new(),
            command_history_cursor: None,
            should_quit: false,
        }
    }

    async fn refresh(&mut self) -> Result<usize, ClientError> {
        let result = self.read_current_target().await;
        self.view.refresh_count = self.view.refresh_count.saturating_add(1);
        self.last_refresh = Some(Instant::now());
        self.view.refresh_age = "now".to_string();

        match result {
            Ok(data) => {
                let rows = data.len();
                self.view.data = data;
                self.view.status = DashboardStatus::Connected;
                self.view.message = format!("Refresh OK: {rows} values");
                Ok(rows)
            }
            Err(error) => {
                self.view.status = DashboardStatus::Error;
                self.view.message = format!("Refresh failed: {error}");
                Err(error)
            }
        }
    }

    async fn read_current_target(&self) -> Result<DashboardData, ClientError> {
        let unit = UnitId(self.view.unit_id);
        match self.view.target {
            DashboardTarget::HoldingRegisters => self
                .client
                .read_holding_registers(unit, self.view.address, self.view.quantity)
                .await
                .map(DashboardData::Registers),
            DashboardTarget::InputRegisters => self
                .client
                .read_input_registers(unit, self.view.address, self.view.quantity)
                .await
                .map(DashboardData::Registers),
            DashboardTarget::Coils => self
                .client
                .read_coils(unit, self.view.address, self.view.quantity)
                .await
                .map(DashboardData::Bits),
            DashboardTarget::DiscreteInputs => self
                .client
                .read_discrete_inputs(unit, self.view.address, self.view.quantity)
                .await
                .map(DashboardData::Bits),
        }
    }

    fn should_auto_refresh(&self) -> bool {
        if self.refresh_interval.is_zero() {
            return false;
        }
        self.last_refresh
            .is_some_and(|last_refresh| last_refresh.elapsed() >= self.refresh_interval)
    }

    fn needs_initial_refresh(&self) -> bool {
        self.last_refresh.is_none()
    }

    fn update_refresh_age(&mut self) {
        self.view.refresh_age = self
            .last_refresh
            .map(format_elapsed)
            .unwrap_or_else(|| "not refreshed".to_string());
    }

    async fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.view.command_mode == CommandMode::Editing {
            self.handle_command_key(key).await;
            return false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
                false
            }
            KeyCode::Char(':') => {
                self.view.command_mode = CommandMode::Editing;
                self.view.command_input.clear();
                false
            }
            KeyCode::Char('r') => true,
            KeyCode::Char('1') => self.set_target(DashboardTarget::HoldingRegisters),
            KeyCode::Char('2') => self.set_target(DashboardTarget::InputRegisters),
            KeyCode::Char('3') => self.set_target(DashboardTarget::Coils),
            KeyCode::Char('4') => self.set_target(DashboardTarget::DiscreteInputs),
            KeyCode::Tab | KeyCode::Down => self.set_target(self.view.target.next()),
            KeyCode::BackTab | KeyCode::Up => self.set_target(self.view.target.previous()),
            KeyCode::PageDown | KeyCode::Right => {
                self.view.address = self.view.address.saturating_add(self.view.quantity.max(1));
                true
            }
            KeyCode::PageUp | KeyCode::Left => {
                self.view.address = self.view.address.saturating_sub(self.view.quantity.max(1));
                true
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.view.quantity = self.view.quantity.saturating_add(1);
                self.view.clamp_quantity();
                true
            }
            KeyCode::Char('-') => {
                self.view.quantity = self.view.quantity.saturating_sub(1).max(1);
                true
            }
            _ => false,
        }
    }

    async fn handle_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.view.command_mode = CommandMode::Idle;
                self.view.command_input.clear();
                self.command_history_cursor = None;
            }
            KeyCode::Enter => {
                let command = std::mem::take(&mut self.view.command_input);
                self.view.command_mode = CommandMode::Idle;
                self.command_history_cursor = None;
                self.execute_command_line(command).await;
            }
            KeyCode::Up => self.recall_previous_command(),
            KeyCode::Down => self.recall_next_command(),
            KeyCode::Backspace => {
                self.view.command_input.pop();
                self.command_history_cursor = None;
            }
            KeyCode::Char(c) => {
                self.view.command_input.push(c);
                self.command_history_cursor = None;
            }
            _ => {}
        }
    }

    async fn execute_command_line(&mut self, line: String) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        self.push_history(trimmed);
        self.push_log(CommandLogEntry::info(format!("> {trimmed}")));

        match shell_parser::parse_command(trimmed) {
            Ok(command) => self.execute_shell_command(command).await,
            Err(error) => self.record_error(format!("Parse error: {error}")),
        }
    }

    async fn execute_shell_command(&mut self, command: ShellCommand) {
        match command {
            ShellCommand::ReadCoils { address, quantity } => {
                self.read_target(DashboardTarget::Coils, address, quantity)
                    .await;
            }
            ShellCommand::ReadDiscreteInputs { address, quantity } => {
                self.read_target(DashboardTarget::DiscreteInputs, address, quantity)
                    .await;
            }
            ShellCommand::ReadHoldingRegisters { address, quantity } => {
                self.read_target(DashboardTarget::HoldingRegisters, address, quantity)
                    .await;
            }
            ShellCommand::ReadInputRegisters { address, quantity } => {
                self.read_target(DashboardTarget::InputRegisters, address, quantity)
                    .await;
            }
            ShellCommand::WriteCoil { address, value } => {
                let result = self
                    .client
                    .write_single_coil(UnitId(self.view.unit_id), address, value)
                    .await;
                self.finish_write(result, DashboardTarget::Coils, address, 1)
                    .await;
            }
            ShellCommand::WriteCoils { address, values } => {
                let quantity = u16::try_from(values.len()).unwrap_or(u16::MAX);
                let result = self
                    .client
                    .write_multiple_coils(UnitId(self.view.unit_id), address, &values)
                    .await;
                self.finish_write(result, DashboardTarget::Coils, address, quantity)
                    .await;
            }
            ShellCommand::WriteRegister { address, value } => {
                let result = self
                    .client
                    .write_single_register(UnitId(self.view.unit_id), address, value)
                    .await;
                self.finish_write(result, DashboardTarget::HoldingRegisters, address, 1)
                    .await;
            }
            ShellCommand::WriteRegisters { address, values } => {
                let quantity = u16::try_from(values.len()).unwrap_or(u16::MAX);
                let result = self
                    .client
                    .write_multiple_registers(UnitId(self.view.unit_id), address, &values)
                    .await;
                self.finish_write(result, DashboardTarget::HoldingRegisters, address, quantity)
                    .await;
            }
            ShellCommand::SetUnitId(id) => {
                self.view.unit_id = id;
                self.push_log(CommandLogEntry::success(format!("Unit ID set to {id}")));
                let _ = self.refresh().await;
            }
            ShellCommand::Status => {
                self.push_log(CommandLogEntry::info(format!(
                    "Endpoint {} unit {} connected {}",
                    self.view.endpoint,
                    self.view.unit_id,
                    self.client.is_connected()
                )));
            }
            ShellCommand::Help => {
                for line in [
                    "read holding-registers <address> <quantity>",
                    "read coils <address> <quantity>",
                    "write register <address> <value>",
                    "write coil <address> <on|off>",
                ] {
                    self.push_log(CommandLogEntry::info(line));
                }
            }
            ShellCommand::Exit => {
                self.should_quit = true;
            }
            ShellCommand::Empty => {}
        }
    }

    async fn read_target(&mut self, target: DashboardTarget, address: u16, quantity: u16) {
        self.view.target = target;
        self.view.address = address;
        self.view.quantity = quantity;
        self.view.clamp_quantity();

        match self.refresh().await {
            Ok(rows) => self.push_log(CommandLogEntry::success(format!(
                "Read {rows} {} values from {address}",
                target.short_label()
            ))),
            Err(error) => self.push_log(CommandLogEntry::error(format!("Read failed: {error}"))),
        }
    }

    async fn finish_write(
        &mut self,
        result: Result<(), ClientError>,
        target: DashboardTarget,
        address: u16,
        quantity: u16,
    ) {
        match result {
            Ok(()) => {
                self.push_log(CommandLogEntry::success(format!(
                    "Write OK: {quantity} {} values at {address}",
                    target.short_label()
                )));
                self.view.target = target;
                self.view.address = address;
                self.view.quantity = quantity.max(1);
                self.view.clamp_quantity();
                let _ = self.refresh().await;
            }
            Err(error) => self.record_error(format!("Write failed: {error}")),
        }
    }

    fn record_error(&mut self, message: String) {
        self.view.status = DashboardStatus::Error;
        self.view.message = message.clone();
        self.push_log(CommandLogEntry::error(message));
    }

    fn push_log(&mut self, entry: CommandLogEntry) {
        if self.view.command_log.len() == MAX_COMMAND_LOG {
            self.view.command_log.pop_front();
        }
        self.view.command_log.push_back(entry);
    }

    fn push_history(&mut self, command: &str) {
        if self
            .command_history
            .back()
            .is_some_and(|last| last == command)
        {
            return;
        }
        if self.command_history.len() == MAX_COMMAND_HISTORY {
            self.command_history.pop_front();
        }
        self.command_history.push_back(command.to_string());
    }

    fn recall_previous_command(&mut self) {
        if self.command_history.is_empty() {
            return;
        }

        let index = self
            .command_history_cursor
            .and_then(|index| index.checked_sub(1))
            .unwrap_or(self.command_history.len() - 1);
        self.command_history_cursor = Some(index);
        self.view.command_input = self.command_history[index].clone();
    }

    fn recall_next_command(&mut self) {
        let Some(index) = self.command_history_cursor else {
            return;
        };

        if index + 1 == self.command_history.len() {
            self.command_history_cursor = None;
            self.view.command_input.clear();
            return;
        }

        let next = index + 1;
        self.command_history_cursor = Some(next);
        self.view.command_input = self.command_history[next].clone();
    }

    fn set_target(&mut self, target: DashboardTarget) -> bool {
        if self.view.target == target {
            return false;
        }
        self.view.target = target;
        self.view.clamp_quantity();
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandMode {
    Idle,
    Editing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandLogStatus {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandLogEntry {
    status: CommandLogStatus,
    text: String,
}

impl CommandLogEntry {
    fn info(text: impl Into<String>) -> Self {
        Self::new(CommandLogStatus::Info, text)
    }

    fn success(text: impl Into<String>) -> Self {
        Self::new(CommandLogStatus::Success, text)
    }

    fn error(text: impl Into<String>) -> Self {
        Self::new(CommandLogStatus::Error, text)
    }

    fn new(status: CommandLogStatus, text: impl Into<String>) -> Self {
        Self {
            status,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardView {
    endpoint: String,
    unit_id: u8,
    timeout_secs: u64,
    target: DashboardTarget,
    address: u16,
    quantity: u16,
    data: DashboardData,
    status: DashboardStatus,
    refresh_count: u64,
    refresh_age: String,
    message: String,
    command_mode: CommandMode,
    command_input: String,
    command_log: VecDeque<CommandLogEntry>,
}

impl DashboardView {
    fn clamp_quantity(&mut self) {
        self.quantity = self.quantity.clamp(1, self.target.max_quantity());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DashboardStatus {
    Connected,
    Error,
}

impl DashboardStatus {
    const fn label(&self) -> &'static str {
        match self {
            Self::Connected => "CONNECTED",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DashboardData {
    Registers(Vec<u16>),
    Bits(Vec<bool>),
    Empty,
}

impl DashboardData {
    fn len(&self) -> usize {
        match self {
            Self::Registers(values) => values.len(),
            Self::Bits(values) => values.len(),
            Self::Empty => 0,
        }
    }
}

fn format_elapsed(instant: Instant) -> String {
    let elapsed = instant.elapsed();
    if elapsed.as_secs() == 0 {
        "now".to_string()
    } else {
        format!("{}s ago", elapsed.as_secs())
    }
}

#[cfg(test)]
mod tests;
