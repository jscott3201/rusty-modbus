use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

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
    }
}

fn render_to_buffer(view: &DashboardView, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_dashboard(frame, view))
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
