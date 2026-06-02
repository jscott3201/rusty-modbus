use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_sim::{ModbusSimulator, generic_io};

use super::render::palette;
use super::*;

fn sample_view(data: DashboardData) -> DashboardView {
    DashboardView {
        endpoint: "127.0.0.1:502".to_string(),
        unit_id: 7,
        timeout_secs: 5,
        target: DashboardTarget::HoldingRegisters,
        address: 400,
        quantity: 4,
        data,
        status: DashboardStatus::Connected,
        refresh_count: 2,
        refresh_age: "now".to_string(),
        message: "Refresh OK: 2 values".to_string(),
        command_mode: CommandMode::Idle,
        command_input: String::new(),
        command_log: VecDeque::from([CommandLogEntry::info(
            "Press ':' to run read/write/status/help commands.",
        )]),
    }
}

fn render_to_buffer(view: &DashboardView, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render::render_dashboard(frame, view))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &Buffer) -> String {
    let mut text = String::new();
    let area = *buffer.area();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

#[test]
fn render_dashboard_shows_endpoint_mode_and_registers() {
    let view = sample_view(DashboardData::Registers(vec![0x1234, 0x0001]));
    let buffer = render_to_buffer(&view, 110, 32);
    let text = buffer_text(&buffer);

    assert!(text.contains("rusty-modbus dashboard"));
    assert!(text.contains("127.0.0.1:502"));
    assert!(text.contains("Holding Registers"));
    assert!(text.contains("0x1234"));
    assert!(text.contains("4660"));
    assert!(text.contains("PageUp/PageDown"));
    assert!(text.contains("Up/Down"));
    assert!(text.contains("COMMAND"));
}

#[test]
fn render_dashboard_uses_industrial_blue_palette() {
    let view = sample_view(DashboardData::Registers(vec![1]));
    let buffer = render_to_buffer(&view, 110, 32);

    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.bg == palette::BACKGROUND)
    );
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.bg == palette::PANEL)
    );
    assert!(buffer.content().iter().any(|cell| cell.fg == palette::CYAN));
}

#[test]
fn render_dashboard_marks_error_state() {
    let mut view = sample_view(DashboardData::Empty);
    view.status = DashboardStatus::Error;
    view.message = "Refresh failed: timed out".to_string();

    let text = buffer_text(&render_to_buffer(&view, 110, 32));

    assert!(text.contains("ERROR"));
    assert!(text.contains("timed out"));
    assert!(text.contains("no data"));
}

#[test]
fn render_compact_dashboard_for_small_terminals() {
    let view = sample_view(DashboardData::Registers(vec![1]));

    let text = buffer_text(&render_to_buffer(&view, 40, 10));

    assert!(text.contains("rusty-modbus dashboard"));
    assert!(text.contains("Resize for full controls"));
}

#[test]
fn render_dashboard_shows_command_input_and_log() {
    let mut view = sample_view(DashboardData::Registers(vec![1]));
    view.command_mode = CommandMode::Editing;
    view.command_input = "read coils 0 8".to_string();
    view.command_log
        .push_back(CommandLogEntry::success("Read 8 CO values from 0"));

    let text = buffer_text(&render_to_buffer(&view, 110, 32));

    assert!(text.contains(":read coils 0 8"));
    assert!(text.contains("Read 8 CO values from 0"));
}

#[test]
fn render_dashboard_shows_all_concise_help_lines() {
    let mut view = sample_view(DashboardData::Registers(vec![1]));
    view.command_log.clear();
    for line in shell_parser::HELP_LINES {
        view.command_log.push_back(CommandLogEntry::info(*line));
    }

    let text = buffer_text(&render_to_buffer(&view, 110, 32));

    for line in shell_parser::HELP_LINES {
        assert!(text.contains(line));
    }
}

#[test]
fn clamps_quantity_by_target_limits() {
    let mut view = sample_view(DashboardData::Empty);

    view.quantity = 0;
    view.clamp_quantity();
    assert_eq!(view.quantity, 1);

    view.quantity = 500;
    view.target = DashboardTarget::HoldingRegisters;
    view.clamp_quantity();
    assert_eq!(view.quantity, 125);

    view.quantity = 500;
    view.target = DashboardTarget::Coils;
    view.clamp_quantity();
    assert_eq!(view.quantity, 500);
}

#[tokio::test]
async fn dashboard_command_reads_registers() {
    let (mut sim, addr) = start_sim().await;
    sim.set_holding_register(0, 0x1234);
    sim.set_holding_register(1, 0x0002);

    let mut app = app_for(addr).await;
    app.execute_command_line("read holding-registers 0 2".to_string())
        .await;

    assert_eq!(app.view.target, DashboardTarget::HoldingRegisters);
    assert_eq!(app.view.address, 0);
    assert_eq!(app.view.quantity, 2);
    assert_eq!(
        app.view.data,
        DashboardData::Registers(vec![0x1234, 0x0002])
    );
    assert!(
        app.view
            .command_log
            .iter()
            .any(|entry| entry.text.contains("Read 2 HR values from 0"))
    );

    sim.stop().await;
}

#[tokio::test]
async fn dashboard_command_writes_register_and_refreshes() {
    let (mut sim, addr) = start_sim().await;

    let mut app = app_for(addr).await;
    app.execute_command_line("write register 0 42".to_string())
        .await;

    assert_eq!(app.view.target, DashboardTarget::HoldingRegisters);
    assert_eq!(app.view.address, 0);
    assert_eq!(app.view.quantity, 1);
    assert_eq!(app.view.data, DashboardData::Registers(vec![42]));
    assert!(
        app.view
            .command_log
            .iter()
            .any(|entry| entry.text.contains("Write OK: 1 HR values at 0"))
    );

    sim.stop().await;
}

#[tokio::test]
async fn dashboard_command_reports_parse_errors() {
    let (mut sim, addr) = start_sim().await;

    let mut app = app_for(addr).await;
    app.execute_command_line("write register 0 nope".to_string())
        .await;

    assert_eq!(app.view.status, DashboardStatus::Error);
    assert!(
        app.view
            .command_log
            .iter()
            .any(|entry| entry.status == CommandLogStatus::Error)
    );

    sim.stop().await;
}

#[tokio::test]
async fn dashboard_command_help_uses_shared_help_lines() {
    let (mut sim, addr) = start_sim().await;
    let mut app = app_for(addr).await;

    app.execute_command_line("help".to_string()).await;

    let log_text = app
        .view
        .command_log
        .iter()
        .map(|entry| entry.text.as_str())
        .collect::<Vec<_>>();
    for help_line in shell_parser::HELP_LINES {
        assert!(log_text.contains(help_line));
    }

    sim.stop().await;
}

#[tokio::test]
async fn dashboard_command_history_navigates_previous_and_next() {
    let (mut sim, addr) = start_sim().await;
    let mut app = app_for(addr).await;

    app.execute_command_line("status".to_string()).await;
    app.execute_command_line("help".to_string()).await;
    app.view.command_mode = CommandMode::Editing;

    app.handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .await;
    assert_eq!(app.view.command_input, "help");

    app.handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .await;
    assert_eq!(app.view.command_input, "status");

    app.handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .await;
    assert_eq!(app.view.command_input, "status");

    app.handle_command_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .await;
    assert_eq!(app.view.command_input, "help");

    app.handle_command_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .await;
    assert!(app.view.command_input.is_empty());

    sim.stop().await;
}

#[tokio::test]
async fn dashboard_command_history_skips_consecutive_duplicates() {
    let (mut sim, addr) = start_sim().await;
    let mut app = app_for(addr).await;

    app.execute_command_line("status".to_string()).await;
    app.execute_command_line("status".to_string()).await;

    assert_eq!(app.command_history.len(), 1);

    sim.stop().await;
}

async fn start_sim() -> (ModbusSimulator, std::net::SocketAddr) {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();
    (sim, addr)
}

async fn app_for(addr: std::net::SocketAddr) -> DashboardApp {
    let client = ModbusClient::connect(
        addr,
        ClientConfig {
            timeout: Duration::from_secs(2),
            ..ClientConfig::default()
        },
    )
    .await
    .unwrap();

    DashboardApp::new(
        DashboardConfig {
            addr,
            unit_id: 1,
            timeout: 2,
            address: 0,
            quantity: 1,
            target: DashboardTarget::HoldingRegisters,
            refresh_interval: Duration::ZERO,
        },
        client,
    )
}
