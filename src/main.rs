mod app;
mod config;
mod model;
mod platform;
mod ui;

use std::process::Command;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use app::Action;

fn main() -> std::io::Result<()> {
    let cfg = match config::Config::load(std::env::args().skip(1)) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            print!("{}", config::USAGE);
            return Ok(());
        }
        Err(e) => {
            eprintln!("ntop: {e}");
            std::process::exit(2);
        }
    };
    let refresh = Duration::from_secs_f64(cfg.interval);
    let mut collector = platform::collector(&cfg);
    let mut app = app::App::new(collector.sample(), &cfg);
    let mut terminal = ratatui::init();
    let mut last = Instant::now();
    while !app.quit {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        if event::poll(refresh.saturating_sub(last.elapsed()))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    if let Some(action) = app.on_key(k.code) {
                        app.status = Some(run_action(action, &mut terminal));
                    }
                }
            }
        }
        if last.elapsed() >= refresh {
            app.update(collector.sample());
            last = Instant::now();
        }
    }
    ratatui::restore();
    Ok(())
}

/// Performs an `Action` and returns the status line to show.
fn run_action(action: Action, terminal: &mut DefaultTerminal) -> String {
    match action {
        Action::Kill(pid) => {
            // SAFETY: plain syscall; a bad pid just returns an error.
            if unsafe { libc::kill(pid as i32, libc::SIGTERM) } == 0 {
                format!("sent SIGTERM to {pid}")
            } else {
                format!("kill {pid} failed: {}", std::io::Error::last_os_error())
            }
        }
        Action::Docker(args) => {
            let label = args.join(" ");
            // `docker stop` can take 10 s; do not block the UI.
            std::thread::spawn(move || Command::new("docker").args(&args).output());
            format!("docker {label} started")
        }
        Action::Logs(id) => {
            ratatui::restore();
            let r = Command::new("sh").arg("-c").arg(format!("docker logs --tail 500 {id} 2>&1 | less -R +G")).status();
            *terminal = ratatui::init();
            match r {
                Ok(_) => format!("showed logs of {id}"),
                Err(e) => format!("docker logs failed: {e}"),
            }
        }
    }
}
