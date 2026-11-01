# ntop

A Windows-Task-Manager-style system monitor for the terminal, in the spirit of `htop`/`btop`,
with first-class Docker awareness. Written in Rust with [ratatui](https://ratatui.rs).
Runs on Linux and macOS.

```
┌ ntop │ myhost │ up 6d 16h │ load 2.65 1.38 0.72 ──────────────────────────────────────────┐
│███████     CPU 11.1% (10 cores)                          ▁▂▃▅▇▇▃▂▁▁▂▃▅▇▇▃▂▁▁▂▃▅▇           │
│██████████  MEM 1.7G / 7.7G  swap 0B / 1.0G                ▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆          │
│ NET  ↓  1.2M/s  ↑ 300.0K/s                                ▂▁▁█▂▁▁▁▂▃▁▁▁▁▂▁▁▁▁▁▁▁▁▁          │
│ DISK read  5.0M/s  write 1.0M/s      210 processes, 5 containers   █▅▁▁▁▁▁▁▂▁▁▁▁▁▁▁▁▁▁▁▁   │
│ CORES ▆▆▆▅▅▄▃▃▂▂                                                                           │
│ GPU0 NVIDIA GeForce RTX 3080 12% 1.0G / 10.0G 45°C   │   TEMP coretemp 52°C  nvme 38°C     │
└────────────────────────────────────────────────────────────────────────────────────────────┘
┌ Processes (210/210) by container ─────────────────────────────────────────── sort: CPU ↓ ┐
│PID     USER       S CPU%  MEM     DISK R/s DISK W/s CONTAINER      NAME      COMMAND       │
│                                                                              ▼ web  ·  3 processes  ·  CPU 10.2%  ·  MEM 45.3M
│48756   root       R 10.0  976.0K  0B/s     0B/s     web            sh        └ sh -c ...   │
│                                                                              ▼ host  ·  207 processes  ·  CPU 1.4%  ·  MEM 1.2G
│331     root       S 0.4   101.4M  0B/s     0B/s                    dockerd   └ /usr/bin/...│
└────────────────────────────────────────────────────────────────────────────────────────────┘
┌ Docker (5/5) ─────────────────────────────────────────────────────────────── sort: CPU ↓ ┐
│ID           NAME        CPU%   MEM      LIMIT    NET RX   NET TX   BLK R    BLK W    PIDS  │
│5728eb74cfd1 web         100.1  544.0K   7.7G     746B     0B       0B       0B       1     │
└────────────────────────────────────────────────────────────────────────────────────────────┘
 filter: web  (Esc clears)
 q quit │ Tab pane │ / filter │ k kill │ t tree │ g group │ sort: c cpu m mem d disk w net n name p pid u user
```

## What it shows

**Header** – CPU, memory/swap gauges with a scrolling history sparkline for CPU, MEM, NET and
DISK; network receive/transmit rate; disk read/write rate; one bar glyph per CPU core; GPU
utilisation / memory / temperature and temperature sensors when the machine exposes them; load
average and uptime.

**Processes** – every process on the machine, optionally as a tree (`t`) or grouped by Docker
container (`g`):

| Column    | Meaning                                                                    |
|-----------|----------------------------------------------------------------------------|
| PID       | process id                                                                 |
| USER      | owner (Linux: `/etc/passwd`; macOS: Directory Services), falls back to the uid |
| S         | state: R running, S sleeping, D uninterruptible, Z zombie, I idle, T stopped |
| CPU%      | share of **total** machine capacity, like Task Manager (all cores = 100 %) |
| MEM       | resident memory                                                            |
| DISK R/s, W/s | bytes actually read from / written to storage per second               |
| NET RX/s, TX/s | per-process network, only with `--per-process-net` (see below)        |
| CONTAINER | Docker container name if the process runs inside one                       |
| NAME      | short process name                                                         |
| COMMAND   | full command line (`[name]` for kernel threads)                            |

**Docker** – every running container, as reported by `docker stats`: CPU % (percent of one core,
can exceed 100), memory used / limit, cumulative network RX/TX, cumulative block read/write,
and the number of processes inside.

## Install

Requires a Rust toolchain (`curl https://sh.rustup.rs -sSf | sh`) and, for the Docker pane,
the `docker` CLI with access to the daemon. Same steps on Linux and macOS:

```sh
git clone https://github.com/ShahafChen/ntop.git
cd ntop
make            # builds target/release/ntop
sudo make install   # copies it to /usr/local/bin/ntop
```

No toolchain on the machine? `make docker-build` builds the Linux binary inside the official
`rust` image (needs Docker).

Other targets: `make run`, `make test`, `make uninstall`, `make clean`.

## Usage

```sh
ntop                              # defaults
ntop --interval 2 --sort mem      # slower refresh, sorted by memory
ntop --tree --hide disk,state     # process tree, fewer columns
ntop --per-process-net            # add NET RX/s + NET TX/s per process (off by default)
ntop --no-docker                  # never run docker, hide the Docker pane
ntop --help
```

Run it as root (`sudo ntop`) to see disk I/O for *other users'* processes; both kernels hide
per-process I/O counters from non-owners, so those rows show `0B/s` otherwise. The Docker pane
needs access to the Docker daemon (Linux: user in the `docker` group, or root; macOS: Docker
Desktop running); if `docker stats` fails the pane title shows why.

### Keys

| Key           | Action                                                              |
|---------------|---------------------------------------------------------------------|
| `q`           | quit (`Esc` quits too, unless a filter is active: then it clears it) |
| `Tab`         | switch the active pane (Processes ↔ Docker, yellow border)          |
| `↑` `↓` `PgUp` `PgDn` `Home` `End` | move the selection                             |
| `/`           | type a filter (matches name, user, command line, container, exact pid); `Enter` keeps it, `Esc` clears |
| `t`           | toggle process tree (parent → children, indented)                   |
| `g`           | toggle grouping by container, with a summary line per container     |
| `k`           | send SIGTERM to the selected process (asks `y/N` first)             |
| `s` / `r`     | Docker pane: stop / restart the selected container (asks `y/N`)     |
| `l`           | Docker pane: open the container's last 500 log lines in `less`      |
| `c` `m` `d` `w` `n` `p` `u` | sort by CPU, memory, disk, network, name, PID, user; the same key again reverses |

Numeric sorts start with the biggest value on top; text sorts start A→Z.
One sort setting applies to both panes; in the Docker pane `d` is block I/O, `w` is network, `p` is PID count.

### Options and config file

Every option can also be set in `~/.config/ntop/config` (or `$XDG_CONFIG_HOME/ntop/config`),
one `key = value` per line, `#` for comments. Flags override the file.

| Flag / key              | Default | Meaning                                                             |
|-------------------------|---------|---------------------------------------------------------------------|
| `--interval <secs>`     | `1`     | refresh period, minimum 0.2                                         |
| `--sort <key>`          | `cpu`   | `cpu`, `mem`, `disk`, `net`, `name`, `pid`, `user`                   |
| `--tree`                | off     | start in tree view                                                  |
| `--group`               | off     | start grouped by container                                          |
| `--filter <text>`       | none    | start with this filter                                              |
| `--hide <cols>`         | none    | comma list of process columns to hide: `pid,user,state,cpu,mem,disk,net,container,name,command` |
| `--no-docker` / `docker = false` | docker on | skip Docker entirely                                       |
| `--per-process-net`     | off     | per-process network columns (see below)                             |
| `--config <path>`       |         | alternative config file                                             |

Example config:

```ini
interval = 2
sort = mem
hide = state, disk
per-process-net = true
```

### Per-process network (opt-in)

Off by default because it costs an extra command per refresh and is approximate:

- **Linux** uses `ss -tinHOp`: TCP sockets only, bytes come from the kernel's `tcp_info`,
  and `ss -p` only shows other users' sockets when run as root. A socket shared by several
  processes is charged to the first one listed.
- **macOS** uses `nettop -P -L 1` (all protocols, no root needed).

## Code layout

```
src/
  main.rs             event loop: sample, handle keys, draw, run actions (kill, docker, logs)
  config.rs           flags + config file → Config
  model.rs            OS-neutral data: SystemStats, Process, Container, Gpu, Snapshot, sorting
  app.rs              UI state (sort, filter, tree/group rows, confirmations, history) and keys
  ui.rs               ratatui rendering
  platform/
    mod.rs            `Collector` trait + `collector()` factory, selects the backend by target OS;
                      also `Counters` + `Rates`: shared "two samples → rates" math
    docker.rs         `docker stats` command runner + parser, polled on a background thread (shared)
    linux/
      mod.rs          LinuxCollector: /proc, /sys/block, /sys/class/hwmon, /etc/passwd, nvidia-smi, ss
      proc.rs         pure parsers for /proc, nvidia-smi and ss text (unit tested, no I/O)
    macos/
      mod.rs          MacCollector: sysctl, vm_stat, netstat -ib, ioreg, ps, dscl, nettop;
                      host_processor_info for per-core CPU, proc_pid_rusage for per-process disk I/O
      cmd.rs          pure parsers for those commands' output (unit tested, no I/O)
```

Everything OS-specific lives in `src/platform/<os>/`. The rest of the program only sees the
`Collector` trait and the structs in `model.rs`. A backend reads raw monotonic counters into a
`Counters` value and hands it to `Rates::apply`, which does all the delta/rate arithmetic.

### Porting to another OS

1. Create `src/platform/<os>/` with a collector implementing `platform::Collector`
   (`docker.rs` is reusable as-is since it only shells out to the `docker` CLI).
2. In `src/platform/mod.rs` add `#[cfg(target_os = "<os>")] pub mod <os>;` and a matching
   `collector()` branch, and extend the `compile_error!` guard.

Nothing in `model.rs`, `app.rs` or `ui.rs` needs to change.

## Limitations

- Container NET and BLK figures are cumulative since the container started (that is what
  `docker stats` reports), not rates.
- The system NET rate skips `lo`, `veth*`, `docker*` and `br-*` so container traffic is not
  counted twice; other virtual interfaces are included.
- GPU: NVIDIA only on Linux (via `nvidia-smi`); on macOS the built-in accelerator's
  utilisation and memory from `ioreg`, no temperature.
- Temperatures: Linux `hwmon` sensors only; macOS needs `sudo powermetrics`, so none are shown.
- Linux: tested on Debian/Ubuntu kernels; cgroup v1 and v2.
- macOS: disk rates come from `ioreg` block-storage statistics and include every attached disk
  (disk images too). The CONTAINER column is always empty because Docker Desktop runs
  containers in a VM; the Docker pane still works.

## License

MIT
