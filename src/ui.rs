//! ratatui rendering. Reads `App`, never mutates data except table scroll state.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph, Row as TRow, Sparkline, Table};
use ratatui::Frame;

use crate::app::{App, Pane, Row};
use crate::model::Process;

const PROC_HELP: &str = " q quit │ Tab pane │ / filter │ k kill │ t tree │ g group │ sort: c cpu m mem d disk w net n name p pid u user (again = reverse) │ ↑↓ PgUp PgDn Home End";
const CONT_HELP: &str = " q quit │ Tab pane │ / filter │ s stop │ r restart │ l logs │ sort: c cpu m mem d disk w net n name p pids (again = reverse) │ ↑↓ PgUp PgDn Home End";

/// (hide key, header, width; 0 = fill)
const PROC_COLS: [(&str, &str, u16); 12] = [
    ("pid", "PID", 7),
    ("user", "USER", 10),
    ("state", "S", 1),
    ("cpu", "CPU%", 5),
    ("mem", "MEM", 7),
    ("disk", "DISK R/s", 8),
    ("disk", "DISK W/s", 8),
    ("net", "NET RX/s", 8),
    ("net", "NET TX/s", 8),
    ("container", "CONTAINER", 14),
    ("name", "NAME", 16),
    ("command", "COMMAND", 0),
];

pub fn draw(f: &mut Frame, app: &mut App) {
    let extra = usize::from(!app.snap.system.gpus.is_empty() || !app.snap.system.temps.is_empty());
    let header_h = 7 + extra as u16; // borders + CPU, MEM, NET, DISK, CORES (+ GPU/TEMP)
    let docker_h = if app.docker { (app.cont_rows.len() as u16 + 3).clamp(3, f.area().height / 3) } else { 0 };
    let [header, procs, docker, footer] =
        Layout::vertical([Constraint::Length(header_h), Constraint::Min(5), Constraint::Length(docker_h), Constraint::Length(2)]).areas(f.area());

    draw_header(f, app, header);
    draw_processes(f, app, procs);
    if app.docker {
        draw_containers(f, app, docker);
    }
    draw_footer(f, app, footer);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap.system;
    let block = Block::bordered().title(format!(" ntop │ {} │ up {} │ load {:.2} {:.2} {:.2} ", s.hostname, uptime(s.uptime_secs), s.load_avg[0], s.load_avg[1], s.load_avg[2]));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical(vec![Constraint::Length(1); inner.height as usize]).split(inner);
    let split = |r: Rect| -> [Rect; 2] { Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(r) };

    let [cpu, cpu_hist] = split(rows[0]);
    f.render_widget(gauge(format!("CPU {:.1}% ({} cores)", s.cpu_pct, s.cpu_cores), s.cpu_pct / 100.0, level_color(s.cpu_pct)), cpu);
    sparkline(f, &app.hist.cpu, Some(1000), level_color(s.cpu_pct), cpu_hist);

    let [mem, mem_hist] = split(rows[1]);
    let mem_pct = ratio(s.mem_used, s.mem_total);
    f.render_widget(gauge(format!("MEM {} / {}  swap {} / {}", bytes(s.mem_used), bytes(s.mem_total), bytes(s.swap_used), bytes(s.swap_total)), mem_pct, level_color(mem_pct * 100.0)), mem);
    sparkline(f, &app.hist.mem, Some(1000), Color::Blue, mem_hist);

    let [net, net_hist] = split(rows[2]);
    f.render_widget(Paragraph::new(format!(" NET  ↓ {:>9}  ↑ {:>9}", rate(s.net_rx_bps), rate(s.net_tx_bps))), net);
    sparkline(f, &app.hist.net, None, Color::Cyan, net_hist);

    let [disk, disk_hist] = split(rows[3]);
    f.render_widget(Paragraph::new(format!(" DISK read {:>9}  write {:>9}      {} processes, {} containers", rate(s.disk_read_bps), rate(s.disk_write_bps), app.snap.processes.len(), app.snap.containers.len())), disk);
    sparkline(f, &app.hist.disk, None, Color::Magenta, disk_hist);

    let mut cores: Vec<Span> = vec![Span::raw(" CORES ")];
    cores.extend(s.core_pct.iter().map(|&p| Span::styled(bar_glyph(p), level_color(p))));
    f.render_widget(Paragraph::new(Line::from(cores)), rows[4]);

    if let Some(row) = rows.get(5) {
        let mut parts: Vec<String> = s
            .gpus
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let mut t = format!("GPU{i} {} {:.0}%", g.name, g.util_pct);
                if g.mem_total > 0 {
                    t += &format!(" {} / {}", bytes(g.mem_used), bytes(g.mem_total));
                } else if g.mem_used > 0 {
                    t += &format!(" {}", bytes(g.mem_used));
                }
                if let Some(c) = g.temp_c {
                    t += &format!(" {c:.0}°C");
                }
                t
            })
            .collect();
        if !s.temps.is_empty() {
            parts.push(format!("TEMP {}", s.temps.iter().map(|(n, c)| format!("{n} {c:.0}°C")).collect::<Vec<_>>().join("  ")));
        }
        f.render_widget(Paragraph::new(format!(" {}", parts.join("   │   "))), *row);
    }
}

fn sparkline(f: &mut Frame, data: &std::collections::VecDeque<u64>, max: Option<u64>, color: Color, area: Rect) {
    let w = area.width as usize;
    let tail: Vec<u64> = data.iter().skip(data.len().saturating_sub(w)).copied().collect();
    let mut sp = Sparkline::default().data(tail).style(color);
    if let Some(m) = max {
        sp = sp.max(m);
    }
    f.render_widget(sp, area);
}

fn bar_glyph(pct: f64) -> &'static str {
    const GLYPHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    GLYPHS[((pct / 100.0 * 7.0).round() as usize).min(7)]
}

fn cell(p: &Process, header: &str, depth: usize) -> String {
    match header {
        "PID" => p.pid.to_string(),
        "USER" => p.user.clone(),
        "S" => p.state.to_string(),
        "CPU%" => format!("{:.1}", p.cpu_pct),
        "MEM" => bytes(p.mem_bytes),
        "DISK R/s" => rate(p.disk_read_bps),
        "DISK W/s" => rate(p.disk_write_bps),
        "NET RX/s" => rate(p.net_rx_bps),
        "NET TX/s" => rate(p.net_tx_bps),
        "CONTAINER" => p.container.clone().unwrap_or_default(),
        "NAME" => p.name.clone(),
        _ => {
            let cmd = if p.cmdline.is_empty() { format!("[{}]", p.name) } else { p.cmdline.clone() };
            if depth == 0 { cmd } else { format!("{}└ {cmd}", "  ".repeat(depth - 1)) }
        }
    }
}

fn draw_processes(f: &mut Frame, app: &mut App, area: Rect) {
    let cols: Vec<&(&str, &str, u16)> = PROC_COLS.iter().filter(|c| !app.hide.contains(c.0)).collect();
    let widths: Vec<Constraint> = cols.iter().map(|c| if c.2 == 0 { Constraint::Min(10) } else { Constraint::Length(c.2) }).collect();
    let rows = app.rows.iter().map(|r| match r {
        Row::Proc(i, depth) => {
            let p = &app.snap.processes[*i];
            TRow::new(cols.iter().map(|c| cell(p, c.1, *depth)).collect::<Vec<_>>())
        }
        Row::Header { name, count, cpu, mem } => {
            let text = format!("▼ {name}  ·  {count} processes  ·  CPU {cpu:.1}%  ·  MEM {}", bytes(*mem));
            TRow::new(cols.iter().map(|c| if c.1 == "COMMAND" { text.clone() } else { String::new() }).collect::<Vec<_>>()).style(Style::new().cyan().bold())
        }
    });
    let shown = app.rows.iter().filter(|r| matches!(r, Row::Proc(..))).count();
    let mut title = format!(" Processes ({shown}/{}) ", app.snap.processes.len());
    if app.tree {
        title += "tree ";
    } else if app.group {
        title += "by container ";
    }
    let table = Table::new(rows, widths)
        .header(TRow::new(cols.iter().map(|c| c.1).collect::<Vec<_>>()).bold().underlined())
        .block(pane_block(title, app, Pane::Processes))
        .row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(table, area, &mut app.proc_state);
}

fn draw_containers(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match &app.snap.docker_error {
        Some(e) => format!(" Docker: unavailable ({e}) "),
        None => format!(" Docker ({}/{}) ", app.cont_rows.len(), app.snap.containers.len()),
    };
    let rows = app.cont_rows.iter().map(|&i| {
        let c = &app.snap.containers[i];
        TRow::new([
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
        .header(TRow::new(header).bold().underlined())
        .block(pane_block(title, app, Pane::Containers))
        .row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(table, area, &mut app.cont_state);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let [line1, line2] = Layout::vertical([Constraint::Length(1); 2]).areas(area);
    let top = if app.editing {
        Line::from(format!(" filter: {}▏  (Enter keep, Esc clear)", app.filter)).yellow()
    } else if let Some((q, _)) = &app.pending {
        Line::from(format!(" {q}")).yellow().bold()
    } else if let Some(s) = &app.status {
        Line::from(format!(" {s}")).green()
    } else if !app.filter.is_empty() {
        Line::from(format!(" filter: {}  (Esc clears)", app.filter)).yellow()
    } else {
        Line::default()
    };
    f.render_widget(Paragraph::new(top), line1);
    let help = if app.pane == Pane::Processes { PROC_HELP } else { CONT_HELP };
    f.render_widget(Paragraph::new(help).dark_gray(), line2);
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
        assert_eq!((bar_glyph(0.0), bar_glyph(40.0), bar_glyph(100.0)), ("▁", "▄", "█"));
    }
}
