# Tray probe — `tray-icon` + `muda` under gpui 0.2.2 (Design 0 of the tray plan)

> Status: **self-driven cases recorded; mechanism sealed on them**
> (2026-09-05). The human cases 2 and 4 were not run before the feature:
> the founder chose to proceed, and those actions are performed on the
> feature itself in the tray plan's human DoD walk (below). The probe was
> `crates/corral-desktop/examples/tray_probe.rs`, carried by commit
> `8f35ff9`, and is deleted with the feature landing (plan D0).
> Protocol and pass criteria: `docs/decisions/2026-09-05-tray-grill.md`
> Q4 and Q9. What it decides: whether the preferred mechanism composes with
> gpui's own `NSApplication` and run loop well enough to carry the accepted
> lifecycle. It does not reopen tray product semantics.

## Method

One gpui application, one plain window, a `tray-icon` status item with a
`muda` menu created inside `Application::run` (the loop is already running
there, the condition `tray-icon` documents for macOS). The menu and tray
callbacks only push a message onto a `futures` unbounded channel; a task
spawned on gpui's foreground awaits the channel and acts. A synthetic
projection steps every 5 s through the ruled scenarios (baseline; an
identical generation that must not rebuild; reorder; disappearance; item id
replaced on the same session; the same session moving Needs You → Ready;
a new session), rebuilding the whole menu and the badge only when the value
changes. Three programmatic close/reopen cycles run 3 s apart, then the
window is closed for good and RSS / CPU (`ps -o rss,pcpu`) and context
switches (`top -stats csw`, as a wakeup proxy) are sampled once a second and
logged every five. The Desktop's own `Bridge` polls the daemon at 1 Hz
throughout. Logs: `+seconds [thread] event`.

Runs: macOS 26.5, debug profile, `target/debug/tray_probe` (the example
copied beside `corrald` so activation resolves its sibling — an example
binary in `target/debug/examples/` has no sibling daemon). Daemon: a
test-support `corrald` under `CORRAL_TEST_ROOT=/tmp/crl-tray` (0700; the
scratchpad path exceeds the Unix socket address limit), reached through
`CORRAL_ENDPOINT`, because the canonical root's registry is schema 1 and
this build refuses it (below).

## Case 1 — status item creation

Created on the first attempt in every run: 141.6 ms cold (first run of the
day, icon and menu classes loading), 27–46 ms on later runs. Exactly one
item appeared per run and it left the menu bar when the process exited
(observed on the self-driven runs' termination by `timeout`). Whether a
second item ever appears across close/reopen is a visual check the human
run confirms; the process holds exactly one `TrayIcon` and never rebuilds
it — only its menu and title.

## Case 3 — windowless lifecycle (self-driven)

Every run: three cycles of `close_window` → `on_window_closed` fired with
`0 window(s) remain; process stays` → `open_window` `opened a new window`,
then the final close into the windowless phase, where the process stayed
alive with the poll running until `timeout` ended it (30–45 s). No cycle
failed to reopen. `on_window_closed` reports the count after removal, so
the lifecycle rule (stay when a tray is established) has the fact it
needs at the moment it needs it.

## Case 5 — dynamic menu update (self-driven)

Generation log from the first run (`tray-probe-self.log`):

```text
+ 0.438s scenario 'initial':      rebuilt gen 1, badge 2, rows [s1=a1, s2=a2, s3=a3]
+ 5.977s scenario 'unchanged':    projection unchanged -> no rebuild (gen 1 stays)
+11.270s scenario 'reorder':      rebuilt gen 2, badge 2, rows [s2=a2, s1=a1, s3=a3]
+16.393s scenario 'disappearance (s1 gone)':          rebuilt gen 3, badge 1, rows [s2=a2, s3=a3]
+21.463s scenario 'item replaced (s2: a2 -> a2-prime)': rebuilt gen 4, badge 3, rows [s1=a1, s2=a2-prime, s3=a3]
+26.753s scenario 'same session changed state (s2 -> Ready, b2)': rebuilt gen 5, badge 3, rows [s1=a1, s3=a3, s2=b2]
+32.085s scenario 'new session (s4)': rebuilt gen 6, badge 3, rows [s1=a1, s2=a2, s4=a4, s3=a3]
+37.391s scenario 'baseline':     rebuilt gen 7, badge 2, rows [s1=a1, s2=a2, s3=a3]
+42.409s scenario 'unchanged':    projection unchanged -> no rebuild (gen 7 stays)
```

Rebuilds happen only on a structural change; `set_menu` + `set_title` on
the live item swapped generations while windowless without error. Each row's
`MenuId` is `session:<id>`, so a click resolves an identity rather than a
position; whether a click on a superseded generation still arrives with the
right id is part of the human run (click a row, wait for the next scenario,
click again).

## Case 6 — idle resources, windowless, real 1 Hz poll

From `tray-probe-self4.log` (daemon answering; `polls ok 26 err 0` at the
end):

| windowless for | RSS | CPU (ps pcpu, decaying avg) | context switches |
|---|---|---|---|
| 1 s | 81.7 MiB | 1.4 % | 61 /s |
| 12 s | 81.9 MiB | 0.9 % | 43 /s |
| 17 s | 81.9 MiB | 0.6 % | 22 /s |

Without a daemon (activation retrying on its backoff) the same process sat
at 81.0–81.8 MiB, 0.2–1.3 %, 16–21 /s (`tray-probe-self.log`). The debug
profile inflates CPU; RSS is dominated by gpui's Metal renderer and font
system, which a windowless process still holds. Recorded, not thresholded
(Q9): one windowless debug process at ~82 MiB and ~1 % CPU is the number
the reconciliation judges.

## Cases 2 and 4 — human run (folded into the feature's DoD walk)

Not run on the probe. The same actions are performed on the Desktop itself,
in an unlocked Aqua session, as the tray plan's human DoD walk. The feature
logs no click, so the walk judges by what each action must visibly do, with
the main window closed and a managed session running:

- click the status item — the menu opens: the header "Needs You N · Ready
  M", the rows, Open Corral, New Session…, Quit Corral;
- Open Corral — one main window comes forward;
- New Session… — the window, with the new-session form open;
- a session row, twice across a generation — the first click selects and
  opens that session; after it has left Needs You / Ready, the second click
  (from a menu opened before it left) brings the window forward on the
  current list and opens nothing else;
- the Dock icon with no window — one main window;
- Quit Corral — the warning that the session will continue running; Cancel
  keeps the item and the process; Quit removes the item and the process
  (`pgrep -x corral-desktop` empty).

Pass requires: one effect per click (a duplicate handler would open two
windows or two forms), no second status item at any point, no click that
reaches another session than the one it named, and the process gone after
Quit. The probe's thread criterion (every callback on `[main]`) is not
observable on the feature and is moot for it: the handler only forwards
the clicked id, and the Watch acts on gpui's foreground whichever thread
delivered it.

## Dependencies

With both crates' default features off (their defaults are the Linux GTK3
and libxdo paths), the lockfile gains nine crates: `tray-icon` 0.24.2,
`muda` 0.19.3, `keyboard-types` 0.7.0, `crossbeam-channel` 0.5.16, `dpi`
0.1.2, `objc2-core-graphics` 0.3.2 — the six that build on macOS — and
`dirs` 6.0.0, `dirs-sys` 0.5.0, `redox_users` 0.5.2, which `tray-icon`
declares for Linux only and no Corral target builds. `cargo deny check` is
green; the Linux graph of `corral-desktop` contains none of them. With the
default features on, the lockfile had carried the gtk3-rs stack and eight
unmaintained advisories (RUSTSEC-2024-0370, -0412, -0413, -0415, -0416,
-0418, -0419, -0420) for crates that never build — the reason the features
are off. On macOS the crates share `objc2` 0.6.4, `objc2-app-kit` 0.3.2
and `objc2-foundation` 0.3.2 with gpui's blade path; no duplicate
Objective-C stack.

## Findings beside the probe

- The canonical root `~/.corral/state/registry.sqlite3` on this machine is
  schema 1 (2026-08-22); every current build's `corrald` exits with
  "the registry store is schema 1; this build knows schema 5", so the
  Desktop and the probe cannot activate a daemon there. `STORAGE_EPOCH` is
  `dev`, under which the database is disposable; deleting it is the
  founder's call, not the probe's.
- A stale test double from an earlier repro is still listening on
  `/tmp/p1repro-42629/run/corrald.sock` (a Python process that accepts and
  never answers). Unrelated to the probe; noted so it is not mistaken for
  a daemon.
