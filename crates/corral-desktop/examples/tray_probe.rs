//! Design 0 of the tray plan: does `tray-icon` + `muda` compose with gpui's
//! own NSApplication and run loop? (`docs/plans/2026-09-05-tray.md` D0;
//! grill Q4/Q9.) Disposable: it proves the mechanism and is deleted when
//! the feature lands.
//!
//! What it drives on its own: a status item with a menu; a synthetic
//! projection that changes every 5 s (reorder, disappearance, item
//! replacement, a session changing state, a new session); three
//! programmatic close/reopen cycles of the window; a real 1 Hz poll of the
//! daemon through the Desktop's own bridge; RSS / CPU / context-switch
//! sampling while windowless. What the founder drives: clicking the status
//! item, Open Corral, New Session…, Quit Corral, and the Dock icon. Every
//! event lands in the log with the thread it arrived on.
//!
//! Run: `cargo run -p corral-desktop --example tray_probe`; the log path is
//! printed first (override with `TRAY_PROBE_LOG`).
#![forbid(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("tray_probe is a macOS probe (tray plan D0)");
}

#[cfg(target_os = "macos")]
fn main() {
    probe::main();
}

#[cfg(target_os = "macos")]
mod probe {
    use std::cell::RefCell;
    use std::fs::File;
    use std::io::Write;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::thread::ThreadId;
    use std::time::{Duration, Instant, SystemTime};

    use corral_client::{ClientActivationPolicy, EndpointSelection};
    use corral_desktop::bridge::Bridge;
    use futures::StreamExt;
    use futures::channel::mpsc::{UnboundedSender, unbounded};
    use gpui::prelude::*;
    use gpui::{
        App, AppContext, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions,
        div, point, px, size,
    };
    use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    const POLL: Duration = Duration::from_secs(1);
    const SCENARIO_STEP: Duration = Duration::from_secs(5);
    const CYCLE_STEP: Duration = Duration::from_secs(3);
    const CYCLES: usize = 3;

    // ---- log -------------------------------------------------------------

    static LOG: Mutex<Option<(File, Instant, ThreadId)>> = Mutex::new(None);

    fn log(line: &str) {
        let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((file, started, main)) = guard.as_mut() {
            let thread = std::thread::current();
            let here = if thread.id() == *main {
                "main".to_owned()
            } else {
                format!("{:?}/{}", thread.id(), thread.name().unwrap_or("?"))
            };
            let stamp = format!("+{:>8.3}s [{here}] {line}", started.elapsed().as_secs_f64());
            let _ = writeln!(file, "{stamp}");
            let _ = file.flush();
            eprintln!("{stamp}");
        }
    }

    // ---- synthetic projection -------------------------------------------

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Row {
        session: &'static str,
        item: &'static str,
        acknowledged: bool,
        title: &'static str,
        since_secs: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Projection {
        needs_you: Vec<Row>,
        ready: Vec<Row>,
    }

    impl Projection {
        fn badge(&self) -> usize {
            self.needs_you
                .iter()
                .chain(&self.ready)
                .filter(|r| !r.acknowledged)
                .count()
        }
    }

    fn row(
        session: &'static str,
        item: &'static str,
        acknowledged: bool,
        title: &'static str,
        since_secs: u64,
    ) -> Row {
        Row {
            session,
            item,
            acknowledged,
            title,
            since_secs,
        }
    }

    /// The scenarios grill Q9 asks for, in order, then repeated. Generation
    /// 1 equals generation 0 on purpose: an unchanged projection must not
    /// rebuild.
    fn scenario(step: usize) -> (&'static str, Projection) {
        let base = || Projection {
            needs_you: vec![
                row("s1", "a1", false, "fix the flaky test", 30),
                row("s2", "a2", true, "write the migration", 400),
            ],
            ready: vec![row("s3", "a3", false, "review PR 53", 5000)],
        };
        match step % 7 {
            0 => ("baseline", base()),
            1 => ("unchanged (must not rebuild)", base()),
            2 => {
                let mut p = base();
                p.needs_you.reverse();
                ("reorder", p)
            }
            3 => {
                let mut p = base();
                p.needs_you.remove(0);
                ("disappearance (s1 gone)", p)
            }
            4 => {
                let mut p = base();
                p.needs_you[1] = row("s2", "a2-prime", false, "write the migration", 3);
                ("item replaced (s2: a2 -> a2-prime, unacknowledged)", p)
            }
            5 => {
                let mut p = base();
                let moved = p.needs_you.remove(1);
                p.ready
                    .push(row(moved.session, "b2", false, moved.title, 1));
                (
                    "same session changed state (s2: Needs You -> Ready, item b2)",
                    p,
                )
            }
            _ => {
                let mut p = base();
                p.needs_you
                    .push(row("s4", "a4", false, "new session appeared", 0));
                ("new session (s4)", p)
            }
        }
    }

    fn age_bucket(secs: u64) -> String {
        match secs {
            ..60 => "<1m".to_owned(),
            60..3_600 => format!("{}m", secs / 60),
            3_600..172_800 => format!("{}h", secs / 3_600),
            _ => format!("{}d", secs / 86_400),
        }
    }

    // ---- menu generation --------------------------------------------------

    fn build_menu(p: &Projection, generation: usize) -> Result<Menu, muda::Error> {
        let menu = Menu::new();
        let header = format!("Needs You {} · Ready {}", p.needs_you.len(), p.ready.len());
        menu.append(&MenuItem::with_id("header", header, false, None))?;
        menu.append(&PredefinedMenuItem::separator())?;
        for (label, rows) in [("Needs You", &p.needs_you), ("Ready", &p.ready)] {
            if rows.is_empty() {
                continue;
            }
            menu.append(&MenuItem::with_id(
                format!("group:{label}"),
                label,
                false,
                None,
            ))?;
            for r in rows {
                let marker = if r.acknowledged { "   " } else { "•  " };
                let text = format!(
                    "{marker}{} · {label} · {}",
                    r.title,
                    age_bucket(r.since_secs)
                );
                let id = format!("session:{}", r.session);
                menu.append(&MenuItem::with_id(id, text, true, None))?;
            }
        }
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&MenuItem::with_id("open", "Open Corral", true, None))?;
        menu.append(&MenuItem::with_id("new", "New Session…", true, None))?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&MenuItem::with_id(
            "quit",
            format!("Quit Corral (probe gen {generation})"),
            true,
            None,
        ))?;
        Ok(menu)
    }

    fn icon() -> Icon {
        // A 36×36 template glyph: a ring open on the right, black with alpha.
        let n = 36u32;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        let c = (n as f32 - 1.0) / 2.0;
        for y in 0..n {
            for x in 0..n {
                let dx = x as f32 - c;
                let dy = y as f32 - c;
                let d = (dx * dx + dy * dy).sqrt();
                let ring = (9.0..=15.0).contains(&d) && !(dx > 6.0 && dy.abs() < 5.0);
                let a = if ring { 255 } else { 0 };
                rgba.extend_from_slice(&[0, 0, 0, a]);
            }
        }
        Icon::from_rgba(rgba, n, n).unwrap_or_else(|e| panic!("a well-formed icon: {e}"))
    }

    // ---- state -------------------------------------------------------------

    struct Probe {
        tray: Option<TrayIcon>,
        projection: Option<Projection>,
        generation: usize,
        window: Option<gpui::AnyWindowHandle>,
        windowless_since: Option<Instant>,
        bridge: Rc<Bridge>,
        polls_ok: usize,
        polls_err: usize,
        events_seen: usize,
    }

    #[derive(Debug)]
    enum Event {
        Menu(MenuId),
        Tray(String),
    }

    struct View;

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .child("Corral tray probe")
                .child("Close this window: the status item must stay and the process must not exit.")
                .child("Then click the status item, Open Corral, New Session…, the Dock icon, and finally Quit Corral.")
        }
    }

    fn open_window(state: &Rc<RefCell<Probe>>, cx: &mut App, why: &str) {
        let already = {
            let s = state.borrow();
            s.window.filter(|w| cx.windows().contains(w))
        };
        if let Some(handle) = already {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log(&format!("open_window({why}): reused the existing window"));
            return;
        }
        let bounds = Bounds {
            origin: point(px(120.), px(120.)),
            size: size(px(640.), px(240.)),
        };
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| View),
        );
        match opened {
            Ok(handle) => {
                let mut s = state.borrow_mut();
                s.window = Some(handle.into());
                s.windowless_since = None;
                log(&format!("open_window({why}): opened a new window"));
            }
            Err(e) => log(&format!("open_window({why}): FAILED: {e}")),
        }
        cx.activate(true);
    }

    fn close_window(state: &Rc<RefCell<Probe>>, cx: &mut App, why: &str) {
        let handle = state.borrow().window;
        match handle {
            Some(h) if cx.windows().contains(&h) => {
                let _ = h.update(cx, |_, window, _| window.remove_window());
                log(&format!("close_window({why}): removed"));
            }
            _ => log(&format!("close_window({why}): no window to close")),
        }
    }

    fn apply_projection(state: &Rc<RefCell<Probe>>, label: &str, p: Projection) {
        let mut s = state.borrow_mut();
        if s.projection.as_ref() == Some(&p) {
            log(&format!(
                "scenario '{label}': projection unchanged -> no rebuild (gen {} stays)",
                s.generation
            ));
            return;
        }
        s.generation += 1;
        let generation = s.generation;
        let menu = match build_menu(&p, generation) {
            Ok(m) => m,
            Err(e) => {
                log(&format!("scenario '{label}': menu build FAILED: {e}"));
                return;
            }
        };
        let badge = p.badge();
        let ids: Vec<String> = p
            .needs_you
            .iter()
            .chain(&p.ready)
            .map(|r| format!("{}={}", r.session, r.item))
            .collect();
        if let Some(tray) = &s.tray {
            tray.set_menu(Some(Box::new(menu)));
            tray.set_title(if badge == 0 {
                None
            } else {
                Some(badge.to_string())
            });
        }
        s.projection = Some(p);
        log(&format!(
            "scenario '{label}': rebuilt gen {generation}, badge {badge}, rows [{}]",
            ids.join(", ")
        ));
    }

    fn handle_event(state: &Rc<RefCell<Probe>>, event: Event, cx: &mut App) {
        {
            let mut s = state.borrow_mut();
            s.events_seen += 1;
        }
        match event {
            Event::Tray(what) => log(&format!("tray event delivered to gpui foreground: {what}")),
            Event::Menu(id) => {
                let id = id.0;
                let generation = state.borrow().generation;
                log(&format!(
                    "menu event delivered to gpui foreground: id={id} (current gen {generation})"
                ));
                match id.as_str() {
                    "open" => open_window(state, cx, "Open Corral"),
                    "new" => {
                        open_window(state, cx, "New Session…");
                        log("New Session…: would open the Desktop's ⌘N form");
                    }
                    "quit" => {
                        log("Quit Corral: dropping the tray, then quitting in 1 s");
                        state.borrow_mut().tray = None;
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(Duration::from_secs(1)).await;
                            log("quit");
                            let _ = cx.update(|cx| cx.quit());
                        })
                        .detach();
                    }
                    other => match other.strip_prefix("session:") {
                        Some(session) => {
                            let present = state.borrow().projection.as_ref().is_some_and(|p| {
                                p.needs_you
                                    .iter()
                                    .chain(&p.ready)
                                    .any(|r| r.session == session)
                            });
                            if present {
                                open_window(state, cx, &format!("row {session}"));
                                log(&format!(
                                    "row click resolved session {session}: in current gen {generation} -> would select and Open"
                                ));
                            } else {
                                log(&format!(
                                    "row click resolved session {session}: NOT in current gen {generation} -> converge, no action"
                                ));
                            }
                        }
                        None => log(&format!("menu id ignored: {other}")),
                    },
                }
            }
        }
    }

    fn sample(pid: u32) -> String {
        let ps = std::process::Command::new("ps")
            .args(["-o", "rss=,pcpu=", "-p", &pid.to_string()])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        let mut parts = ps.split_whitespace();
        let rss_kib: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let cpu = parts.next().unwrap_or("?");
        format!("rss {:.1} MiB, cpu {cpu}%", rss_kib as f64 / 1024.0)
    }

    fn context_switches(pid: u32) -> Option<u64> {
        let out = std::process::Command::new("top")
            .args(["-l", "1", "-pid", &pid.to_string(), "-stats", "csw"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().last()?.trim().parse().ok()
    }

    pub fn main() {
        let path = std::env::var("TRAY_PROBE_LOG").unwrap_or_else(|_| "tray-probe.log".to_owned());
        let file = match File::create(&path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("tray probe: cannot open {path}: {e}");
                std::process::exit(2);
            }
        };
        *LOG.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((file, Instant::now(), std::thread::current().id()));
        eprintln!("tray probe log: {path}");
        log(&format!(
            "pid {} main thread {:?}",
            std::process::id(),
            std::thread::current().id()
        ));

        let policy = ClientActivationPolicy::resolve();
        let endpoint = match EndpointSelection::from_environment() {
            Ok(endpoint) => endpoint,
            Err(e) => {
                eprintln!("tray probe: {e}");
                std::process::exit(2);
            }
        };
        let bridge = Rc::new(Bridge::start(policy, endpoint));

        let state = Rc::new(RefCell::new(Probe {
            tray: None,
            projection: None,
            generation: 0,
            window: None,
            windowless_since: None,
            bridge,
            polls_ok: 0,
            polls_err: 0,
            events_seen: 0,
        }));

        // Case 4: Dock reopen through gpui's own callback, registered on the
        // Application before the loop runs.
        let reopen_state = state.clone();
        let application = Application::new();
        application.on_reopen(move |cx| {
            log("on_reopen fired (Dock)");
            open_window(&reopen_state, cx, "Dock reopen");
        });
        application.run(move |cx: &mut App| {
            // Case 1: the status item, created inside gpui's running loop.
            let started = Instant::now();
            let built = TrayIconBuilder::new()
                .with_icon(icon())
                .with_icon_as_template(true)
                .with_tooltip("Corral (probe)")
                .build();
            match built {
                Ok(tray) => {
                    state.borrow_mut().tray = Some(tray);
                    log(&format!("case 1: status item created in {:?}", started.elapsed()));
                }
                Err(e) => {
                    log(&format!("case 1: status item creation FAILED: {e} -> the non-watchful lifecycle would apply"));
                }
            }

            // Case 2: the event bridge. Handlers only send; gpui handles.
            let (tx, mut rx): (UnboundedSender<Event>, _) = unbounded();
            {
                let tx = tx.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    log(&format!("menu callback fired: id={}", event.id.0));
                    let _ = tx.unbounded_send(Event::Menu(event.id));
                }));
            }
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let what = match &event {
                    TrayIconEvent::Click { button, button_state, .. } => format!("click {button:?} {button_state:?}"),
                    TrayIconEvent::DoubleClick { .. } => "double click".to_owned(),
                    other => format!("{other:?}").chars().take(40).collect(),
                };
                if matches!(event, TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. }) {
                    log(&format!("tray callback fired: {what}"));
                    let _ = tx.unbounded_send(Event::Tray(what));
                }
            }));
            {
                let state = state.clone();
                cx.spawn(async move |cx| {
                    while let Some(event) = rx.next().await {
                        let state = state.clone();
                        let _ = cx.update(|cx| handle_event(&state, event, cx));
                    }
                })
                .detach();
            }

            // Closing the window by hand must not quit: log and stay.
            {
                let state = state.clone();
                cx.on_window_closed(move |cx| {
                    let open = cx.windows().len();
                    log(&format!("window closed; {open} window(s) remain; process stays"));
                    if open == 0 {
                        state.borrow_mut().windowless_since = Some(Instant::now());
                    }
                })
                .detach();
            }

            apply_projection(&state, "initial", scenario(0).1);
            open_window(&state, cx, "startup");

            // The real 1 Hz poll through the Desktop's bridge.
            {
                let state = state.clone();
                cx.spawn(async move |cx| {
                    loop {
                        let bridge = state.borrow().bridge.clone();
                        let reply = bridge.poll();
                        let outcome = reply.await;
                        {
                            let mut s = state.borrow_mut();
                            match outcome {
                                Ok(Ok(polled)) => {
                                    if s.polls_ok == 0 {
                                        log(&format!(
                                            "poll answered: {} sessions, Needs You {} · Ready {}",
                                            polled.listing.items.len(),
                                            polled.summary.needs_you.total,
                                            polled.summary.ready.total
                                        ));
                                    }
                                    s.polls_ok += 1;
                                }
                                Ok(Err(unanswered)) => {
                                    if s.polls_err < 3 {
                                        log(&format!("poll unanswered: {}", unanswered.line()));
                                    }
                                    s.polls_err += 1;
                                }
                                Err(_) => {
                                    log("poll: the bridge stopped");
                                    s.polls_err += 1;
                                }
                            }
                        }
                        cx.background_executor().timer(POLL).await;
                    }
                })
                .detach();
            }

            // Case 5: the projection changes every 5 s.
            {
                let state = state.clone();
                cx.spawn(async move |cx| {
                    let mut step = 1;
                    loop {
                        cx.background_executor().timer(SCENARIO_STEP).await;
                        let (label, p) = scenario(step);
                        apply_projection(&state, label, p);
                        step += 1;
                    }
                })
                .detach();
            }

            // Case 3: programmatic close/reopen cycles, then windowless.
            {
                let state = state.clone();
                cx.spawn(async move |cx| {
                    for cycle in 1..=CYCLES {
                        cx.background_executor().timer(CYCLE_STEP).await;
                        let _ = cx.update(|cx| close_window(&state, cx, &format!("cycle {cycle}")));
                        cx.background_executor().timer(CYCLE_STEP).await;
                        let _ = cx.update(|cx| open_window(&state, cx, &format!("cycle {cycle}")));
                    }
                    cx.background_executor().timer(CYCLE_STEP).await;
                    let _ = cx.update(|cx| close_window(&state, cx, "entering the windowless phase"));
                    log("windowless phase: click the status item, the menu items, and the Dock icon; Quit Corral ends the probe");
                })
                .detach();
            }

            // Case 6: resources, sampled while windowless.
            {
                let state = state.clone();
                let pid = std::process::id();
                cx.spawn(async move |cx| {
                    let mut csw_start: Option<(u64, Instant)> = None;
                    let mut ticks = 0u64;
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let (windowless, ok, err, events) = {
                            let s = state.borrow();
                            (s.windowless_since, s.polls_ok, s.polls_err, s.events_seen)
                        };
                        let Some(since) = windowless else {
                            csw_start = None;
                            continue;
                        };
                        ticks += 1;
                        if csw_start.is_none() {
                            csw_start = context_switches(pid).map(|c| (c, Instant::now()));
                        }
                        if ticks.is_multiple_of(5) {
                            let csw = match (csw_start, context_switches(pid)) {
                                (Some((c0, t0)), Some(c1)) => {
                                    format!("{:.1} csw/s", (c1.saturating_sub(c0)) as f64 / t0.elapsed().as_secs_f64())
                                }
                                _ => "csw n/a".to_owned(),
                            };
                            log(&format!(
                                "windowless {:>4}s: {}, {csw}; polls ok {ok} err {err}; events {events}",
                                since.elapsed().as_secs(),
                                sample(pid)
                            ));
                        }
                    }
                })
                .detach();
            }
            let _ = SystemTime::now();
        });
    }
}
