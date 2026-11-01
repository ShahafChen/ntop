# ntop

A Windows-Task-Manager-style system monitor for the terminal, in the spirit of `htop`/`btop`,
with first-class Docker awareness. Written in Rust with [ratatui](https://ratatui.rs).
Runs on Linux and macOS.

```
┌ ntop │ myhost │ up 6d 16h │ load 2.65 1.38 0.72 ─────────────────────────────────────────┐
│███████     CPU 11.1% (10 cores)      ██████████  MEM 1.7G / 7.7G        SWAP 0B / 1.0G    │
│ NET ↓  1.2M/s  ↑ 300.0K/s   │   DISK read  5.0M/s  write 1.0M/s   │   210 processes, 5 containers
└───────────────────────────────────────────────────────────────────────────────────────────┘
┌ Processes (210) ──────────────────────────────────────────────────────────── sort: CPU ↓ ┐
│PID     USER       S CPU%  MEM     DISK R/s DISK W/s CONTAINER      NAME      COMMAND      │
│48756   root       R 10.0  976.0K  0B/s     0B/s     web            sh        sh -c ...    │
│331     root       S 0.4   101.4M  0B/s     0B/s                    dockerd   /usr/bin/... │
└───────────────────────────────────────────────────────────────────────────────────────────┘
┌ Docker (5) ───────────────────────────────────────────────────────────────── sort: CPU ↓ ┐
│ID           NAME        CPU%   MEM      LIMIT    NET RX   NET TX   BLK R    BLK W    PIDS │
│5728eb74cfd1 web         100.1  544.0K   7.7G     746B     0B       0B       0B       1    │
└───────────────────────────────────────────────────────────────────────────────────────────┘
 q quit │ Tab pane │ ↑↓ PgUp PgDn Home End scroll │ sort: c cpu  m mem  d disk  w net  n name  p pid  u user
```

## What it shows

**Header** – overall CPU %, memory and swap gauges, network receive/transmit rate, disk read/write
rate, load average, uptime.

**Processes** – every process on the machine:

| Column    | Meaning                                                                    |
|-----------|----------------------------------------------------------------------------|
| PID       | process id                                                                 |
| USER      | owner (resolved from `/etc/passwd`, falls back to the uid)                 |
| S         | state: R running, S sleeping, D uninterruptible, Z zombie, I idle, T stopped |
| CPU%      | share of **total** machine capacity, like Task Manager (all cores = 100 %) |
| MEM       | resident memory                                                            |
| DISK R/s, W/s | bytes actually read from / written to storage per second               |
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
ntop
```

Run it as root (`sudo ntop`) to see disk I/O for *other users'* processes; both kernels hide
per-process I/O counters from non-owners, so those rows show `0B/s` otherwise. The Docker pane
needs access to the Docker daemon (Linux: user in the `docker` group, or root; macOS: Docker
Desktop running); if `docker stats` fails the pane title shows why.

The screen refreshes once per second.

### Keys

| Key           | Action                                                         |
|---------------|----------------------------------------------------------------|
| `q`, `Esc`    | quit                                                           |
| `Tab`         | switch the active pane (Processes ↔ Docker, yellow border)     |
| `↑` `↓`       | move the selection one row                                     |
| `PgUp` `PgDn` | move 20 rows                                                   |
| `Home` `End`  | jump to first / last row                                       |
| `c`           | sort by CPU                                                    |
| `m`           | sort by memory                                                 |
| `d`           | sort by disk (processes: read+write rate; containers: block I/O) |
| `w`           | sort by network (containers only; processes keep their order)  |
| `n`           | sort by name                                                   |
| `p`           | sort by PID (containers: by PID count)                         |
| `u`           | sort by user (containers: by name)                             |
| same key / `r`| reverse the current sort direction                             |

Numeric sorts start with the biggest value on top; text sorts start A→Z.
One sort setting applies to both panes.

## Code layout

```
src/
  main.rs             event loop: sample every second, handle keys, draw
  model.rs            OS-neutral data: SystemStats, Process, Container, Snapshot, sorting
  app.rs              UI state (sort key, active pane, scroll) and key handling
  ui.rs               ratatui rendering
  platform/
    mod.rs            `Collector` trait + `collector()` factory, selects the backend by target OS
                        also `Counters` + `Rates`: shared "two samples → rates" math
    docker.rs         `docker stats` command runner + parser, polled on a background thread (shared)
    linux/
      mod.rs          LinuxCollector: reads /proc, /sys/block, /etc/passwd
      proc.rs         pure parsers for /proc text (unit tested, no I/O)
    macos/
      mod.rs          MacCollector: runs sysctl, vm_stat, netstat -ib, ioreg, ps, dscl; proc_pid_rusage for per-process disk I/O
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

- Per-process **network** usage is not available from `/proc` without eBPF/`nethogs`-style
  capture, so the processes table has no network column; network is shown system-wide and
  per container.
- Container NET and BLK figures are cumulative since the container started (that is what
  `docker stats` reports), not rates.
- The system NET rate skips `lo`, `veth*`, `docker*` and `br-*` so container traffic is not
  counted twice; other virtual interfaces are included.
- Linux: tested on Debian/Ubuntu kernels; cgroup v1 and v2.
- macOS: overall CPU % is the sum of per-process CPU time, so time spent by processes that
  exited between two samples is not counted. Disk rates come from `ioreg` block-storage
  statistics and include every attached disk (disk images too). The CONTAINER column is always
  empty because Docker Desktop runs containers in a VM; the Docker pane still works.

## License

MIT
