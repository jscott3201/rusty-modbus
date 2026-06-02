//! Ratatui dashboard for interactive Modbus diagnostics.

use std::error::Error;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_types::UnitId;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    /// Target address.
    pub addr: SocketAddr,
    /// Initial unit ID.
    pub unit_id: u8,
    /// Request timeout in seconds.
    pub timeout: u64,
    /// Initial starting address.
    pub address: u16,
    /// Initial quantity.
    pub quantity: u16,
    /// Initial data area.
    pub target: DashboardTarget,
    /// Auto-refresh interval.
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
    app.refresh().await;

    let mut terminal = TerminalSession::enter()?;
    run_loop(terminal.terminal_mut(), &mut app).await
}

async fn run_loop(
    terminal: &mut DefaultTerminal,
    app: &mut DashboardApp,
) -> Result<(), Box<dyn Error>> {
    while !app.should_quit {
        app.update_refresh_age();
        terminal.draw(|frame| render_dashboard(frame, &app.view))?;

        if app.should_auto_refresh() {
            app.refresh().await;
            continue;
        }

        let Some(Event::Key(key)) = read_event(EVENT_POLL_INTERVAL)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if app.handle_key(key) {
            app.refresh().await;
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
        };
        view.clamp_quantity();

        Self {
            client,
            view,
            refresh_interval: config.refresh_interval,
            last_refresh: None,
            should_quit: false,
        }
    }

    async fn refresh(&mut self) {
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
            }
            Err(error) => {
                self.view.status = DashboardStatus::Error;
                self.view.message = format!("Refresh failed: {error}");
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

    fn update_refresh_age(&mut self) {
        self.view.refresh_age = self
            .last_refresh
            .map(format_elapsed)
            .unwrap_or_else(|| "not refreshed".to_string());
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
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

    fn set_target(&mut self, target: DashboardTarget) -> bool {
        if self.view.target == target {
            return false;
        }
        self.view.target = target;
        self.view.clamp_quantity();
        true
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

    const fn style(&self) -> Style {
        match self {
            Self::Connected => Style::new().fg(palette::GREEN).add_modifier(Modifier::BOLD),
            Self::Error => Style::new().fg(palette::RED).add_modifier(Modifier::BOLD),
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

fn render_dashboard(frame: &mut Frame, view: &DashboardView) {
    let area = frame.area();
    if area.width < 72 || area.height < 20 {
        render_compact(frame, area, view);
        return;
    }

    frame.render_widget(
        Block::new().style(Style::new().bg(palette::BACKGROUND)),
        area,
    );

    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .areas(area);

    render_header(frame, header_area, view);

    let [sidebar_area, data_area] =
        Layout::horizontal([Constraint::Length(31), Constraint::Fill(1)]).areas(body_area);
    render_sidebar(frame, sidebar_area, view);
    render_data_panel(frame, data_area, view);
    render_footer(frame, footer_area);
}

fn render_compact(frame: &mut Frame, area: Rect, view: &DashboardView) {
    let text = Text::from(vec![
        Line::from(Span::styled(
            "rusty-modbus dashboard",
            Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Endpoint: {}", view.endpoint)),
        Line::from(format!(
            "Unit: {}  View: {}",
            view.unit_id,
            view.target.label()
        )),
        Line::from(format!("Status: {}", view.status.label())),
        Line::from("Resize for full controls. q/Esc quits."),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(palette::TEXT).bg(palette::BACKGROUND))
            .wrap(Wrap { trim: true })
            .block(panel("DASHBOARD")),
        area,
    );
}

fn render_header(frame: &mut Frame, area: Rect, view: &DashboardView) {
    let title = Line::from(vec![
        Span::styled(
            "rusty-modbus",
            Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" dashboard", Style::new().fg(palette::TEXT)),
        Span::styled("  |  ", Style::new().fg(palette::MUTED)),
        Span::styled(view.status.label(), view.status.style()),
    ]);
    let metadata = Line::from(vec![
        Span::styled("Endpoint ", Style::new().fg(palette::MUTED)),
        Span::styled(&view.endpoint, Style::new().fg(palette::TEXT)),
        Span::styled("  Unit ", Style::new().fg(palette::MUTED)),
        Span::styled(view.unit_id.to_string(), Style::new().fg(palette::TEXT)),
        Span::styled("  Timeout ", Style::new().fg(palette::MUTED)),
        Span::styled(
            format!("{}s", view.timeout_secs),
            Style::new().fg(palette::TEXT),
        ),
        Span::styled("  Refresh ", Style::new().fg(palette::MUTED)),
        Span::styled(&view.refresh_age, Style::new().fg(palette::AMBER)),
    ]);

    frame.render_widget(
        Paragraph::new(vec![title, metadata]).block(panel("CONTROL")),
        area,
    );
}

fn render_sidebar(frame: &mut Frame, area: Rect, view: &DashboardView) {
    let items = DashboardTarget::ALL
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let selected = *target == view.target;
            let marker = if selected { ">" } else { " " };
            let style = if selected {
                Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(palette::TEXT)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{marker} {}", index + 1),
                    Style::new().fg(palette::MUTED),
                ),
                Span::raw(" "),
                Span::styled(target.short_label(), style),
                Span::styled("  ", Style::new().fg(palette::MUTED)),
                Span::styled(target.label(), style),
            ]))
        })
        .collect::<Vec<_>>();

    let status = Text::from(vec![
        Line::from(vec![
            Span::styled("Mode ", Style::new().fg(palette::MUTED)),
            Span::styled(view.target.function_code(), Style::new().fg(palette::AMBER)),
        ]),
        Line::from(vec![
            Span::styled("Address ", Style::new().fg(palette::MUTED)),
            Span::styled(view.address.to_string(), Style::new().fg(palette::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("Quantity ", Style::new().fg(palette::MUTED)),
            Span::styled(view.quantity.to_string(), Style::new().fg(palette::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("Reads ", Style::new().fg(palette::MUTED)),
            Span::styled(
                view.refresh_count.to_string(),
                Style::new().fg(palette::TEXT),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(&view.message, Style::new().fg(palette::AMBER))),
    ]);

    let [modes_area, status_area] =
        Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area);

    frame.render_widget(
        List::new(items)
            .block(panel("AREAS"))
            .style(Style::new().bg(palette::PANEL)),
        modes_area,
    );
    frame.render_widget(
        Paragraph::new(status)
            .wrap(Wrap { trim: true })
            .block(panel("STATUS")),
        status_area,
    );
}

fn render_data_panel(frame: &mut Frame, area: Rect, view: &DashboardView) {
    let [tabs_area, table_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
    let tabs = Tabs::new(DashboardTarget::ALL.iter().map(|target| target.label()))
        .select(view.target.tab_index())
        .style(Style::new().fg(palette::MUTED).bg(palette::PANEL))
        .highlight_style(Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD))
        .divider("|")
        .block(panel("DATA"));
    frame.render_widget(tabs, tabs_area);

    let rows = data_rows(view);
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(["Address", "Value", "Decimal", "State"])
            .style(Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD))
            .bottom_margin(1),
    )
    .column_spacing(2)
    .block(panel(view.target.label()))
    .style(Style::new().fg(palette::TEXT).bg(palette::PANEL))
    .row_highlight_style(Style::new().bg(palette::STEEL));
    frame.render_widget(table, table_area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer = Line::from(vec![
        Span::styled("q/Esc", Style::new().fg(palette::CYAN)),
        Span::raw(" quit  "),
        Span::styled("r", Style::new().fg(palette::CYAN)),
        Span::raw(" refresh  "),
        Span::styled("1-4/Tab", Style::new().fg(palette::CYAN)),
        Span::raw(" area  "),
        Span::styled("PageUp/PageDown", Style::new().fg(palette::CYAN)),
        Span::raw(" address  "),
        Span::styled("+/-", Style::new().fg(palette::CYAN)),
        Span::raw(" quantity"),
    ]);
    frame.render_widget(
        Paragraph::new(footer)
            .style(Style::new().fg(palette::TEXT).bg(palette::BACKGROUND))
            .block(panel("KEYS")),
        area,
    );
}

fn data_rows(view: &DashboardView) -> Vec<Row<'static>> {
    match &view.data {
        DashboardData::Registers(values) => values
            .iter()
            .enumerate()
            .map(|(offset, value)| {
                Row::new(vec![
                    Cell::from(display_address(view.address, offset).to_string()),
                    Cell::from(format!("0x{value:04X}")),
                    Cell::from(value.to_string()),
                    Cell::from("register"),
                ])
            })
            .collect(),
        DashboardData::Bits(values) => values
            .iter()
            .enumerate()
            .map(|(offset, value)| {
                let state = if *value { "ON" } else { "OFF" };
                Row::new(vec![
                    Cell::from(display_address(view.address, offset).to_string()),
                    Cell::from(if *value { "1" } else { "0" }),
                    Cell::from(if *value { "1" } else { "0" }),
                    Cell::from(state),
                ])
            })
            .collect(),
        DashboardData::Empty => vec![Row::new(vec![
            Cell::from("-"),
            Cell::from("-"),
            Cell::from("-"),
            Cell::from("no data"),
        ])],
    }
}

fn panel(title: &'static str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Plain)
        .title(title)
        .border_style(Style::new().fg(palette::STEEL))
        .style(Style::new().fg(palette::TEXT).bg(palette::PANEL))
}

fn display_address(address: u16, offset: usize) -> u32 {
    u32::from(address).saturating_add(u32::try_from(offset).unwrap_or(u32::MAX))
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
