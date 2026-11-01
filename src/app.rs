//! UI state and key handling; no I/O. Side effects are returned as `Action`s for `main` to run.

use std::collections::{HashMap, HashSet, VecDeque};

use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::TableState;

use crate::config::Config;
use crate::model::{sort_containers, sort_processes, Process, Snapshot, SortKey};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Processes,
    Containers,
}

/// Something `main` has to do on the real system.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Kill(u32),
    /// `docker <args...>`, run in the background.
    Docker(Vec<String>),
    /// Show `docker logs` of this container id in a pager.
    Logs(String),
}

/// One line of the process table.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// Container header when grouping: name, process count, CPU %, memory bytes.
    Header { name: String, count: usize, cpu: f64, mem: u64 },
    /// Index into `snap.processes` and indent depth (tree / group).
    Proc(usize, usize),
}

/// Last `HISTORY` samples for the header sparklines.
#[derive(Default)]
pub struct History {
    pub cpu: VecDeque<u64>,
    pub mem: VecDeque<u64>,
    pub net: VecDeque<u64>,
    pub disk: VecDeque<u64>,
}

const HISTORY: usize = 300;

pub struct App {
    pub snap: Snapshot,
    pub sort: SortKey,
    pub reverse: bool,
    pub pane: Pane,
    pub proc_state: TableState,
    pub cont_state: TableState,
    pub quit: bool,
    pub filter: String,
    pub editing: bool,
    pub tree: bool,
    pub group: bool,
    pub hide: HashSet<String>,
    pub docker: bool,
    /// Question shown in the footer; `y` runs the action.
    pub pending: Option<(String, Action)>,
    pub status: Option<String>,
    pub rows: Vec<Row>,
    /// Indices into `snap.containers` that pass the filter.
    pub cont_rows: Vec<usize>,
    pub hist: History,
}

impl App {
    pub fn new(snap: Snapshot, cfg: &Config) -> Self {
        let mut hide = cfg.hide.clone();
        if !cfg.per_process_net {
            hide.insert("net".to_string());
        }
        let mut app = Self {
            snap: Snapshot::default(),
            sort: cfg.sort,
            reverse: false,
            pane: Pane::Processes,
            proc_state: TableState::default().with_selected(0),
            cont_state: TableState::default().with_selected(0),
            quit: false,
            filter: cfg.filter.clone(),
            editing: false,
            tree: cfg.tree,
            group: cfg.group && !cfg.tree,
            hide,
            docker: cfg.docker,
            pending: None,
            status: None,
            rows: Vec::new(),
            cont_rows: Vec::new(),
            hist: History::default(),
        };
        app.update(snap);
        app
    }

    pub fn update(&mut self, snap: Snapshot) {
        self.snap = snap;
        let s = &self.snap.system;
        let pct = |a: u64, b: u64| (a * 1000).checked_div(b).unwrap_or(0);
        for (q, v) in [
            (&mut self.hist.cpu, (s.cpu_pct * 10.0) as u64),
            (&mut self.hist.mem, pct(s.mem_used, s.mem_total)),
            (&mut self.hist.net, (s.net_rx_bps + s.net_tx_bps) as u64),
            (&mut self.hist.disk, (s.disk_read_bps + s.disk_write_bps) as u64),
        ] {
            q.push_back(v);
            if q.len() > HISTORY {
                q.pop_front();
            }
        }
        self.rebuild();
    }

    /// Re-sort, re-filter and re-layout (tree / groups) the visible rows.
    fn rebuild(&mut self) {
        sort_processes(&mut self.snap.processes, self.sort, self.reverse);
        sort_containers(&mut self.snap.containers, self.sort, self.reverse);
        let needle = self.filter.to_lowercase();
        let hit = |p: &Process| {
            needle.is_empty()
                || p.name.to_lowercase().contains(&needle)
                || p.user.to_lowercase().contains(&needle)
                || p.cmdline.to_lowercase().contains(&needle)
                || p.container.as_deref().is_some_and(|c| c.to_lowercase().contains(&needle))
                || p.pid.to_string() == needle
        };
        let idxs: Vec<usize> = (0..self.snap.processes.len()).filter(|&i| hit(&self.snap.processes[i])).collect();
        self.rows = if self.tree {
            tree_rows(&self.snap.processes, &idxs)
        } else if self.group {
            group_rows(&self.snap.processes, &idxs)
        } else {
            idxs.into_iter().map(|i| Row::Proc(i, 0)).collect()
        };
        self.cont_rows = (0..self.snap.containers.len())
            .filter(|&i| needle.is_empty() || self.snap.containers[i].name.to_lowercase().contains(&needle) || self.snap.containers[i].id.starts_with(&needle))
            .collect();
        clamp(&mut self.proc_state, self.rows.len());
        clamp(&mut self.cont_state, self.cont_rows.len());
    }

    pub fn selected_process(&self) -> Option<&Process> {
        match self.rows.get(self.proc_state.selected()?)? {
            Row::Proc(i, _) => self.snap.processes.get(*i),
            Row::Header { .. } => None,
        }
    }

    pub fn selected_container(&self) -> Option<&crate::model::Container> {
        self.snap.containers.get(*self.cont_rows.get(self.cont_state.selected()?)?)
    }

    pub fn on_key(&mut self, key: KeyCode) -> Option<Action> {
        if self.editing {
            match key {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.editing = false;
                }
                KeyCode::Enter => self.editing = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.rebuild();
            return None;
        }
        if let Some((_, action)) = self.pending.take() {
            return matches!(key, KeyCode::Char('y') | KeyCode::Char('Y')).then_some(action);
        }
        self.status = None;
        let sort = match key {
            KeyCode::Char('q') => return self.quit(),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                None
            }
            KeyCode::Esc => return self.quit(),
            KeyCode::Tab if self.docker => {
                self.pane = if self.pane == Pane::Processes { Pane::Containers } else { Pane::Processes };
                return None;
            }
            KeyCode::Char('/') => {
                self.editing = true;
                return None;
            }
            KeyCode::Char('t') => {
                self.tree = !self.tree;
                self.group = false;
                None
            }
            KeyCode::Char('g') => {
                self.group = !self.group;
                self.tree = false;
                None
            }
            KeyCode::Char('k') if self.pane == Pane::Processes => {
                if let Some(p) = self.selected_process() {
                    self.pending = Some((format!("Send SIGTERM to {} ({})? [y/N]", p.pid, p.name), Action::Kill(p.pid)));
                }
                return None;
            }
            KeyCode::Char('s') | KeyCode::Char('r') if self.pane == Pane::Containers => {
                let verb = if key == KeyCode::Char('s') { "stop" } else { "restart" };
                if let Some(c) = self.selected_container() {
                    self.pending = Some((format!("{verb} container {}? [y/N]", c.name), Action::Docker(vec![verb.to_string(), c.id.clone()])));
                }
                return None;
            }
            KeyCode::Char('l') if self.pane == Pane::Containers => return self.selected_container().map(|c| Action::Logs(c.id.clone())),
            KeyCode::Char('c') => Some(SortKey::Cpu),
            KeyCode::Char('m') => Some(SortKey::Mem),
            KeyCode::Char('d') => Some(SortKey::Disk),
            KeyCode::Char('w') => Some(SortKey::Net),
            KeyCode::Char('n') => Some(SortKey::Name),
            KeyCode::Char('p') => Some(SortKey::Pid),
            KeyCode::Char('u') => Some(SortKey::User),
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
            Pane::Processes => (&mut self.proc_state, self.rows.len()),
            Pane::Containers => (&mut self.cont_state, self.cont_rows.len()),
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
        self.rebuild();
        None
    }

    fn quit(&mut self) -> Option<Action> {
        self.quit = true;
        None
    }
}

fn clamp(state: &mut TableState, len: usize) {
    let i = state.selected().unwrap_or(0).min(len.saturating_sub(1));
    state.select(Some(i));
}

/// Parent → children, depth-first, keeping the current sort order among siblings.
fn tree_rows(procs: &[Process], idxs: &[usize]) -> Vec<Row> {
    let pids: HashSet<u32> = idxs.iter().map(|&i| procs[i].pid).collect();
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for &i in idxs {
        let p = &procs[i];
        if p.ppid != p.pid && pids.contains(&p.ppid) {
            children.entry(p.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }
    fn walk(i: usize, depth: usize, procs: &[Process], children: &HashMap<u32, Vec<usize>>, out: &mut Vec<Row>) {
        out.push(Row::Proc(i, depth));
        for &c in children.get(&procs[i].pid).map(Vec::as_slice).unwrap_or_default() {
            walk(c, depth + 1, procs, children, out);
        }
    }
    let mut out = Vec::with_capacity(idxs.len());
    for i in roots {
        walk(i, 0, procs, &children, &mut out);
    }
    out
}

/// One header per container (host processes under "host"), groups ordered by first appearance.
fn group_rows(procs: &[Process], idxs: &[usize]) -> Vec<Row> {
    let mut order: Vec<&str> = Vec::new();
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for &i in idxs {
        let key = procs[i].container.as_deref().unwrap_or("host");
        if !groups.contains_key(key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(i);
    }
    let mut out = Vec::with_capacity(idxs.len() + order.len());
    for key in order {
        let members = &groups[key];
        out.push(Row::Header {
            name: key.to_string(),
            count: members.len(),
            cpu: members.iter().map(|&i| procs[i].cpu_pct).sum(),
            mem: members.iter().map(|&i| procs[i].mem_bytes).sum(),
        });
        out.extend(members.iter().map(|&i| Row::Proc(i, 1)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        let p = |pid, ppid, name: &str, mem, container: Option<&str>| Process { pid, ppid, name: name.into(), mem_bytes: mem, container: container.map(String::from), ..Default::default() };
        Snapshot {
            processes: vec![p(1, 0, "init", 1, None), p(2, 1, "nginx", 30, Some("web")), p(3, 2, "worker", 20, Some("web")), p(4, 1, "bash", 5, None)],
            ..Default::default()
        }
    }

    #[test]
    fn keys_sort_and_scroll() {
        let mut app = App::new(snap(), &Config::default());
        app.on_key(KeyCode::Char('m'));
        assert_eq!((app.sort, app.reverse, &app.rows[0]), (SortKey::Mem, false, &Row::Proc(0, 0)));
        assert_eq!(app.snap.processes[0].pid, 2);
        app.on_key(KeyCode::Char('m'));
        assert_eq!((app.reverse, app.snap.processes[0].pid), (true, 1));
        app.on_key(KeyCode::End);
        assert_eq!(app.proc_state.selected(), Some(3));
        app.on_key(KeyCode::Down);
        assert_eq!(app.proc_state.selected(), Some(3));
        app.on_key(KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn filter_tree_group() {
        let mut app = App::new(snap(), &Config::default());
        app.on_key(KeyCode::Char('/'));
        for c in "web".chars() {
            app.on_key(KeyCode::Char(c));
        }
        app.on_key(KeyCode::Enter);
        let pids: Vec<u32> = app.rows.iter().filter_map(|r| match r { Row::Proc(i, _) => Some(app.snap.processes[*i].pid), _ => None }).collect();
        assert_eq!(pids, [2, 3]);
        app.on_key(KeyCode::Esc); // clears the filter, does not quit
        assert!(!app.quit && app.filter.is_empty() && app.rows.len() == 4);

        app.on_key(KeyCode::Char('t'));
        let depths: Vec<(u32, usize)> = app.rows.iter().map(|r| match r { Row::Proc(i, d) => (app.snap.processes[*i].pid, *d), _ => unreachable!() }).collect();
        assert_eq!(depths, [(1, 0), (2, 1), (3, 2), (4, 1)]);

        app.on_key(KeyCode::Char('g'));
        assert!(!app.tree && app.group);
        assert_eq!(app.rows[0], Row::Header { name: "host".into(), count: 2, cpu: 0.0, mem: 6 });
        assert_eq!(app.rows[3], Row::Header { name: "web".into(), count: 2, cpu: 0.0, mem: 50 });
        app.on_key(KeyCode::Char('k')); // header row selected: nothing to kill
        assert!(app.pending.is_none());
        app.on_key(KeyCode::Down);
        app.on_key(KeyCode::Char('k'));
        assert!(app.pending.is_some());
        assert_eq!(app.on_key(KeyCode::Char('n')), None); // anything but y cancels
        app.on_key(KeyCode::Char('k'));
        assert_eq!(app.on_key(KeyCode::Char('y')), Some(Action::Kill(1)));
    }
}
