use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
};

use super::{
    CommandLogStatus, CommandMode, DashboardData, DashboardStatus, DashboardTarget, DashboardView,
};

pub(super) mod palette {
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

pub(super) fn render_dashboard(frame: &mut Frame, view: &DashboardView) {
    let area = frame.area();
    if area.width < 72 || area.height < 24 {
        render_compact(frame, area, view);
        return;
    }

    frame.render_widget(
        Block::new().style(Style::new().bg(palette::BACKGROUND)),
        area,
    );

    let [header_area, body_area, command_area, footer_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Fill(1),
        Constraint::Length(7),
        Constraint::Length(3),
    ])
    .areas(area);

    render_header(frame, header_area, view);

    let [sidebar_area, data_area] =
        Layout::horizontal([Constraint::Length(31), Constraint::Fill(1)]).areas(body_area);
    render_sidebar(frame, sidebar_area, view);
    render_data_panel(frame, data_area, view);
    render_command_panel(frame, command_area, view);
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
        Span::styled(view.status.label(), status_style(&view.status)),
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

fn render_command_panel(frame: &mut Frame, area: Rect, view: &DashboardView) {
    let prompt = if view.command_mode == CommandMode::Editing {
        Line::from(vec![
            Span::styled(
                ":",
                Style::new().fg(palette::CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(&view.command_input, Style::new().fg(palette::TEXT)),
        ])
    } else {
        Line::from(vec![
            Span::styled(":", Style::new().fg(palette::MUTED)),
            Span::styled(
                " press ':' for read/write/status/help commands",
                Style::new().fg(palette::MUTED),
            ),
        ])
    };

    let visible_entries = usize::from(area.height.saturating_sub(3));
    let mut lines = vec![prompt];
    for entry in view.command_log.iter().rev().take(visible_entries).rev() {
        lines.push(Line::from(Span::styled(
            &entry.text,
            command_log_style(entry.status),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel("COMMAND")),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer = Line::from(vec![
        Span::styled("q/Esc", Style::new().fg(palette::CYAN)),
        Span::raw(" quit  "),
        Span::styled(":", Style::new().fg(palette::CYAN)),
        Span::raw(" command  "),
        Span::styled("Up/Down", Style::new().fg(palette::CYAN)),
        Span::raw(" history  "),
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

fn command_log_style(status: CommandLogStatus) -> Style {
    match status {
        CommandLogStatus::Info => Style::new().fg(palette::MUTED),
        CommandLogStatus::Success => Style::new().fg(palette::GREEN),
        CommandLogStatus::Error => Style::new().fg(palette::RED),
    }
}

fn status_style(status: &DashboardStatus) -> Style {
    match status {
        DashboardStatus::Connected => Style::new().fg(palette::GREEN).add_modifier(Modifier::BOLD),
        DashboardStatus::Error => Style::new().fg(palette::RED).add_modifier(Modifier::BOLD),
    }
}

fn display_address(address: u16, offset: usize) -> u32 {
    u32::from(address).saturating_add(u32::try_from(offset).unwrap_or(u32::MAX))
}
