//! UI state and key handling; no I/O.

use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::TableState;

use crate::model::{sort_containers, sort_processes, Snapshot, SortKey};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Processes,
    Containers,
}

pub struct App {
    pub snap: Snapshot,
    pub sort: SortKey,
    pub reverse: bool,
    pub pane: Pane,
    pub proc_state: TableState,
    pub cont_state: TableState,
    pub quit: bool,
}

impl App {
    pub fn new(snap: Snapshot) -> Self {
        let mut app = Self {
            snap,
            sort: SortKey::Cpu,
            reverse: false,
            pane: Pane::Processes,
            proc_state: TableState::default().with_selected(0),
            cont_state: TableState::default().with_selected(0),
            quit: false,
        };
        app.resort();
        app
    }

    pub fn update(&mut self, snap: Snapshot) {
        self.snap = snap;
        self.resort();
    }

    fn resort(&mut self) {
        sort_processes(&mut self.snap.processes, self.sort, self.reverse);
        sort_containers(&mut self.snap.containers, self.sort, self.reverse);
        clamp(&mut self.proc_state, self.snap.processes.len());
        clamp(&mut self.cont_state, self.snap.containers.len());
    }

    pub fn on_key(&mut self, key: KeyCode) {
        let sort = match key {
            KeyCode::Char('q') | KeyCode::Esc => return self.quit = true,
            KeyCode::Tab => {
                self.pane = if self.pane == Pane::Processes { Pane::Containers } else { Pane::Processes };
                return;
            }
            KeyCode::Char('c') => Some(SortKey::Cpu),
            KeyCode::Char('m') => Some(SortKey::Mem),
            KeyCode::Char('d') => Some(SortKey::Disk),
            KeyCode::Char('w') => Some(SortKey::Net),
            KeyCode::Char('n') => Some(SortKey::Name),
            KeyCode::Char('p') => Some(SortKey::Pid),
            KeyCode::Char('u') => Some(SortKey::User),
            KeyCode::Char('r') => {
                self.reverse = !self.reverse;
                None
            }
            _ => None,
        };
        if let Some(k) = sort {
            // Same key twice flips the direction.
            if k == self.sort {
                self.reverse = !self.reverse;
            } else {
                self.sort = k;
                self.reverse = false;
            }
        }
        let (state, len) = match self.pane {
            Pane::Processes => (&mut self.proc_state, self.snap.processes.len()),
            Pane::Containers => (&mut self.cont_state, self.snap.containers.len()),
        };
        let i = state.selected().unwrap_or(0);
        let next = match key {
            KeyCode::Up => i.saturating_sub(1),
            KeyCode::Down => i + 1,
            KeyCode::PageUp => i.saturating_sub(20),
            KeyCode::PageDown => i + 20,
            KeyCode::Home => 0,
            KeyCode::End => usize::MAX,
            _ => i,
        };
        state.select(Some(next));
        clamp(state, len);
        self.resort();
    }
}

fn clamp(state: &mut TableState, len: usize) {
    let i = state.selected().unwrap_or(0).min(len.saturating_sub(1));
    state.select(Some(i));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Process;

    #[test]
    fn keys_sort_and_scroll() {
        let snap = Snapshot {
            processes: (0..3).map(|i| Process { pid: i, mem_bytes: i as u64, ..Default::default() }).collect(),
            ..Default::default()
        };
        let mut app = App::new(snap);
        app.on_key(KeyCode::Char('m'));
        assert_eq!((app.sort, app.reverse, app.snap.processes[0].pid), (SortKey::Mem, false, 2));
        app.on_key(KeyCode::Char('m'));
        assert_eq!((app.reverse, app.snap.processes[0].pid), (true, 0));
        app.on_key(KeyCode::End);
        assert_eq!(app.proc_state.selected(), Some(2));
        app.on_key(KeyCode::Down);
        assert_eq!(app.proc_state.selected(), Some(2));
        app.on_key(KeyCode::Char('q'));
        assert!(app.quit);
    }
}
