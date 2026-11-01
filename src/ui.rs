//! ratatui rendering. Reads `App`, never mutates data except table scroll state.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Gauge, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::{App, Pane};

const HELP: &str = " q quit │ Tab pane │ ↑↓ PgUp PgDn Home End scroll │ sort: c cpu  m mem  d disk  w net  n name  p pid  u user  (same key or r = reverse)";

pub fn draw(f: &mut Frame, app: &mut App) {
    let n_cont = app.snap.containers.len() as u16;
    let docker_h = (n_cont + 3).clamp(3, f.area().height / 3);
    let [header, procs, docker, footer] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(5), Constraint::Length(docker_h), Constraint::Length(1)])
            .areas(f.area());

    draw_header(f, app, header);
    draw_processes(f, app, procs);
    draw_containers(f, app, docker);
    f.render_widget(Paragraph::new(HELP).dark_gray(), footer);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap.system;
    let block = Block::bordered().title(format!(" nstop │ {} │ up {} │ load {:.2} {:.2} {:.2} ", s.hostname, uptime(s.uptime_secs), s.load_avg[0], s.load_avg[1], s.load_avg[2]));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [top, bottom] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    let [cpu, mem, swap] = Layout::horizontal([Constraint::Ratio(2, 5), Constraint::Ratio(2, 5), Constraint::Ratio(1, 5)]).areas(top);

    f.render_widget(gauge(format!("CPU {:.1}% ({} cores)", s.cpu_pct, s.cpu_cores), s.cpu_pct / 100.0, level_color(s.cpu_pct)), cpu);
    let mem_pct = ratio(s.mem_used, s.mem_total);
    f.render_widget(gauge(format!("MEM {} / {}", bytes(s.mem_used), bytes(s.mem_total)), mem_pct, level_color(mem_pct * 100.0)), mem);
    f.render_widget(gauge(format!("SWAP {} / {}", bytes(s.swap_used), bytes(s.swap_total)), ratio(s.swap_used, s.swap_total), Color::Magenta), swap);
    f.render_widget(
        Paragraph::new(Line::from(format!(
            " NET ↓ {:>9}  ↑ {:>9}   │   DISK read {:>9}  write {:>9}   │   {} processes, {} containers",
            rate(s.net_rx_bps),
            rate(s.net_tx_bps),
            rate(s.disk_read_bps),
            rate(s.disk_write_bps),
            app.snap.processes.len(),
            app.snap.containers.len()
        ))),
        bottom,
    );
}

fn draw_processes(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.snap.processes.iter().map(|p| {
        Row::new([
            p.pid.to_string(),
            p.user.clone(),
            p.state.to_string(),
            format!("{:.1}", p.cpu_pct),
            bytes(p.mem_bytes),
            rate(p.disk_read_bps),
            rate(p.disk_write_bps),
            p.container.clone().unwrap_or_default(),
            p.name.clone(),
            if p.cmdline.is_empty() { format!("[{}]", p.name) } else { p.cmdline.clone() },
        ])
    });
    let widths = [
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Min(10),
    ];
    let header = ["PID", "USER", "S", "CPU%", "MEM", "DISK R/s", "DISK W/s", "CONTAINER", "NAME", "COMMAND"];
    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .block(pane_block(format!(" Processes ({}) ", app.snap.processes.len()), app, Pane::Processes))
        .row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(table, area, &mut app.proc_state);
}

fn draw_containers(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match &app.snap.docker_error {
        Some(e) => format!(" Docker: unavailable ({e}) "),
        None => format!(" Docker ({}) ", app.snap.containers.len()),
    };
    let rows = app.snap.containers.iter().map(|c| {
        Row::new([
            c.id.clone(),
            c.name.clone(),
            format!("{:.1}", c.cpu_pct),
            bytes(c.mem_bytes),
            bytes(c.mem_limit),
            bytes(c.net_rx_bytes),
            bytes(c.net_tx_bytes),
            bytes(c.blk_read_bytes),
            bytes(c.blk_write_bytes),
            c.pids.to_string(),
        ])
    });
    let widths = [
        Constraint::Length(12),
        Constraint::Min(16),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(5),
    ];
    let header = ["ID", "NAME", "CPU%", "MEM", "LIMIT", "NET RX", "NET TX", "BLK R", "BLK W", "PIDS"];
    let table = Table::new(rows, widths)
        .header(Row::new(header).bold().underlined())
        .block(pane_block(title, app, Pane::Containers))
        .row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(table, area, &mut app.cont_state);
}

fn pane_block(title: String, app: &App, pane: Pane) -> Block<'static> {
    let dir = if app.reverse { "↑" } else { "↓" };
    let b = Block::bordered().title(title).title_top(Line::from(format!(" sort: {} {} ", app.sort.label(), dir)).right_aligned());
    if app.pane == pane { b.border_style(Color::Yellow) } else { b }
}

fn gauge(label: String, ratio: f64, color: Color) -> Gauge<'static> {
    Gauge::default().label(label).ratio(ratio.clamp(0.0, 1.0)).gauge_style(color)
}

fn level_color(pct: f64) -> Color {
    match pct {
        p if p >= 90.0 => Color::Red,
        p if p >= 70.0 => Color::Yellow,
        _ => Color::Green,
    }
}

fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

pub fn bytes(n: u64) -> String {
    human(n as f64, "")
}

pub fn rate(n: f64) -> String {
    human(n, "/s")
}

fn human(mut v: f64, suffix: &str) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0}{}{suffix}", UNITS[i]) } else { format!("{v:.1}{}{suffix}", UNITS[i]) }
}

fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 { format!("{d}d {h}h {m}m") } else { format!("{h}h {m}m") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(bytes(512), "512B");
        assert_eq!(bytes(1536), "1.5K");
        assert_eq!(rate(3.0 * 1024.0 * 1024.0), "3.0M/s");
        assert_eq!(uptime(90_061), "1d 1h 1m");
    }
}
