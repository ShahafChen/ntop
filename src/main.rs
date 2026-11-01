mod app;
mod model;
mod platform;
mod ui;

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};

const REFRESH: Duration = Duration::from_secs(1);

fn main() -> std::io::Result<()> {
    let mut collector = platform::collector();
    let mut app = app::App::new(collector.sample());
    let mut terminal = ratatui::init();
    let mut last = Instant::now();
    while !app.quit {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        if event::poll(REFRESH.saturating_sub(last.elapsed()))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    app.on_key(k.code);
                }
            }
        }
        if last.elapsed() >= REFRESH {
            app.update(collector.sample());
            last = Instant::now();
        }
    }
    ratatui::restore();
    Ok(())
}
