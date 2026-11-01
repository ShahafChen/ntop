//! Settings from `~/.config/ntop/config` (`key = value` lines) overridden by command-line flags.
//! Every key is spelled the same in both places, with `-` or `_` interchangeable.

use std::collections::HashSet;
use std::fs;

use crate::model::SortKey;

pub const USAGE: &str = "\
ntop - task-manager style terminal system monitor

USAGE: ntop [OPTIONS]

  --interval <secs>       refresh period (default 1)
  --sort <key>            cpu | mem | disk | net | name | pid | user (default cpu)
  --tree                  start in process-tree view
  --group                 start grouped by container
  --filter <text>         start with this filter applied
  --hide <cols>           comma list of process columns to hide:
                          pid,user,state,cpu,mem,disk,net,container,name,command
  --no-docker             do not poll docker / show the Docker pane
  --per-process-net       per-process network rates (Linux: ss, TCP only; macOS: nettop)
  --config <path>         config file (default $XDG_CONFIG_HOME/ntop/config or ~/.config/ntop/config)
  -h, --help              this text

The config file takes the same keys, one per line:  interval = 2
";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub interval: f64,
    pub sort: SortKey,
    pub tree: bool,
    pub group: bool,
    pub filter: String,
    pub hide: HashSet<String>,
    pub docker: bool,
    pub per_process_net: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { interval: 1.0, sort: SortKey::Cpu, tree: false, group: false, filter: String::new(), hide: HashSet::new(), docker: true, per_process_net: false }
    }
}

const BOOL_KEYS: [&str; 4] = ["tree", "group", "docker", "per_process_net"];

impl Config {
    /// Config file first, then flags. `Ok(None)` means `--help` was asked for.
    pub fn load(args: impl Iterator<Item = String>) -> Result<Option<Config>, String> {
        let args: Vec<String> = args.collect();
        let path = args
            .iter()
            .position(|a| a == "--config")
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(default_path);
        let mut cfg = Config::default();
        if let Ok(text) = fs::read_to_string(&path) {
            cfg.apply_file(&text).map_err(|e| format!("{path}: {e}"))?;
        }
        if cfg.apply_args(&args)? {
            return Ok(None);
        }
        Ok(Some(cfg))
    }

    fn apply_file(&mut self, text: &str) -> Result<(), String> {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (k, v) = line.split_once('=').ok_or_else(|| format!("bad line: {line}"))?;
            self.set(k.trim(), v.trim())?;
        }
        Ok(())
    }

    /// Returns true when help was requested.
    fn apply_args(&mut self, args: &[String]) -> Result<bool, String> {
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg == "-h" || arg == "--help" {
                return Ok(true);
            }
            let flag = arg.strip_prefix("--").ok_or_else(|| format!("unexpected argument: {arg}\n\n{USAGE}"))?;
            let key = flag.replace('-', "_");
            if key == "config" {
                it.next();
            } else if key == "no_docker" {
                self.set("docker", "false")?;
            } else if let Some((k, v)) = key.split_once('=') {
                self.set(k, v)?;
            } else if BOOL_KEYS.contains(&key.as_str()) {
                self.set(&key, "true")?;
            } else {
                let v = it.next().ok_or_else(|| format!("--{flag} needs a value"))?;
                self.set(&key, v)?;
            }
        }
        Ok(false)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let bool_of = || match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            _ => Err(format!("{key}: expected true/false, got {value}")),
        };
        match key.replace('-', "_").as_str() {
            "interval" => self.interval = value.parse::<f64>().ok().filter(|v| *v >= 0.2).ok_or(format!("interval: must be a number >= 0.2, got {value}"))?,
            "sort" => self.sort = SortKey::parse(value).ok_or(format!("sort: unknown key {value}"))?,
            "tree" => self.tree = bool_of()?,
            "group" => self.group = bool_of()?,
            "filter" => self.filter = value.to_string(),
            "hide" => self.hide = value.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect(),
            "docker" => self.docker = bool_of()?,
            "per_process_net" => self.per_process_net = bool_of()?,
            other => return Err(format!("unknown option: {other}\n\n{USAGE}")),
        }
        Ok(())
    }
}

fn default_path() -> String {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    format!("{base}/ntop/config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_then_flags() {
        let mut cfg = Config::default();
        cfg.apply_file("# comment\ninterval = 2\nsort = mem\nhide = disk, Net\nper-process-net = yes\n").unwrap();
        assert_eq!((cfg.interval, cfg.sort, cfg.per_process_net), (2.0, SortKey::Mem, true));
        assert!(cfg.hide.contains("disk") && cfg.hide.contains("net"));
        let args: Vec<String> = ["--interval=0.5", "--sort", "name", "--tree", "--no-docker", "--filter", "ng"].iter().map(|s| s.to_string()).collect();
        assert!(!cfg.apply_args(&args).unwrap());
        assert_eq!((cfg.interval, cfg.sort, cfg.tree, cfg.docker, cfg.filter.as_str()), (0.5, SortKey::Name, true, false, "ng"));
        assert!(cfg.apply_args(&["--bogus".to_string()]).is_err());
        assert!(cfg.apply_args(&["--interval".to_string(), "0".to_string()]).is_err());
        assert!(cfg.apply_args(&["--help".to_string()]).unwrap());
    }
}
