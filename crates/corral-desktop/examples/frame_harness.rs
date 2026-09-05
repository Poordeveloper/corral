//! The frame harness: the Desktop's own element under a storm, timed.
//!
//! PR9's definition of done asks for the spike's measurement rerun on the
//! real element, in release profile, under a real display link: paint p95
//! within 8 ms at 200×60 (spike grill Q8/Q12). This is that run. It starts a
//! shell that floods its terminal, attaches through the Desktop's bridge,
//! paints through the Desktop's `TerminalElement`, and reports what the
//! paint phase cost.
//!
//! Point it at a daemon with `CORRAL_ENDPOINT` — the one `./scripts/verify`
//! staged under a test root is the honest choice — and run it with the
//! screen unlocked so the display link ticks:
//!
//! ```text
//! cargo run --release -p corral-desktop --example frame_harness -- --size 60x200 --duration 10
//! ```

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use corral_client::launch::{LaunchSite, Requested};
use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_desktop::bridge::{Attached, Bridge};
use corral_desktop::replica::{Geometry, Replica};
use corral_desktop::terminal_element::TerminalElement;
use corral_desktop::theme;
use corral_protocol::terminal::{FrameKind, TerminalFrame};
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    App, Application, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    LayoutId, Pixels, Render, Window, WindowBounds, WindowOptions, div, point, px, size,
};

struct Args {
    rows: u16,
    cols: u16,
    duration: Duration,
    coalesce: Duration,
    command: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        rows: 60,
        cols: 200,
        duration: Duration::from_secs(10),
        coalesce: Duration::from_millis(4),
        command: "yes 'the quick brown fox 汉字 🦊 jumps over the lazy dog 0123456789 \
                  ABCDEFGHIJKLMNOPQRSTUVWXYZ'"
            .to_owned(),
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        let mut value = || words.next().expect("a value");
        match word.as_str() {
            "--size" => {
                let value = value();
                let (rows, cols) = value.split_once('x').expect("RxC");
                args.rows = rows.parse().expect("rows");
                args.cols = cols.parse().expect("cols");
            }
            "--duration" => args.duration = Duration::from_secs(value().parse().expect("seconds")),
            "--coalesce" => {
                args.coalesce = Duration::from_millis(value().parse().expect("milliseconds"));
            }
            "--cmd" => args.command = value(),
            other => panic!("unknown argument {other}"),
        }
    }
    args
}

#[derive(Default)]
struct Stats {
    frames: u64,
    bytes: u64,
    paints: u64,
    paint_us: Vec<u32>,
    latency_us: Vec<u32>,
    ticks: u64,
}

fn percentile(values: &mut [u32], p: f64) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() as f64 - 1.0) * p).round() as usize;
    values[index]
}

struct Harness {
    replica: Replica,
    stats: Rc<RefCell<Stats>>,
    oldest_unpainted: Option<Instant>,
    dirty: bool,
    notify_scheduled: bool,
    coalesce: Duration,
    cache: Option<Rc<qwertty_term_vt::snapshot::SnapshotWindow>>,
}

impl Harness {
    fn new(attached: Attached, coalesce: Duration, cx: &mut Context<Self>) -> Self {
        let mut inbound = attached.inbound;
        let _keep = attached.outbound;
        cx.spawn(async move |this, cx| {
            let _keep = _keep;
            while let Some(delivery) = inbound.next().await {
                let arrived = Instant::now();
                if this
                    .update(cx, |this, cx| this.receive(&delivery.frame, arrived, cx))
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
        Self {
            replica: Replica::new(attached.promised),
            stats: Rc::new(RefCell::new(Stats::default())),
            oldest_unpainted: None,
            dirty: true,
            notify_scheduled: false,
            coalesce,
            cache: None,
        }
    }

    fn receive(&mut self, frame: &TerminalFrame, arrived: Instant, cx: &mut Context<Self>) {
        {
            let mut stats = self.stats.borrow_mut();
            stats.frames += 1;
            stats.bytes += frame.payload.len() as u64;
        }
        let applied = self.replica.apply(frame);
        assert!(!applied.resync, "the harness stream desynchronised");
        if frame.kind == FrameKind::Delta || frame.kind == FrameKind::Snapshot {
            self.dirty = true;
            if self.oldest_unpainted.is_none() {
                self.oldest_unpainted = Some(arrived);
            }
            self.schedule_notify(cx);
        }
    }

    fn schedule_notify(&mut self, cx: &mut Context<Self>) {
        if self.notify_scheduled {
            return;
        }
        self.notify_scheduled = true;
        let coalesce = self.coalesce;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(coalesce).await;
            let _ = this.update(cx, |this, cx| {
                this.notify_scheduled = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Harness {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.dirty {
            self.cache = self.replica.window().ok().map(Rc::new);
            self.dirty = false;
        }
        let stats = Rc::clone(&self.stats);
        window.on_next_frame(move |_, _| {
            stats.borrow_mut().ticks += 1;
        });
        let font = theme::monospace();
        let font_px = px(theme::TERMINAL_FONT_PX);
        let cell = theme::cell_size(window, &font, font_px);
        let element = TerminalElement {
            snapshot: self.cache.clone(),
            font,
            font_px,
            cell,
            // The harness never resizes: the storm stays at the size asked.
            known_grid: Some(Geometry {
                rows: u16::MAX,
                cols: u16::MAX,
            }),
            on_measured: Rc::new(|_, _| {}),
        };
        div().size_full().child(Timed {
            inner: element,
            stats: Rc::clone(&self.stats),
            oldest: self.oldest_unpainted.take(),
        })
    }
}

/// The Desktop's element, with its paint phase timed from outside.
struct Timed {
    inner: TerminalElement,
    stats: Rc<RefCell<Stats>>,
    oldest: Option<Instant>,
}

impl IntoElement for Timed {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Timed {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.inner.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .prepaint(id, inspector, bounds, layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let started = Instant::now();
        self.inner
            .paint(id, inspector, bounds, layout, prepaint, window, cx);
        let mut stats = self.stats.borrow_mut();
        stats.paints += 1;
        stats.paint_us.push(started.elapsed().as_micros() as u32);
        if let Some(arrived) = self.oldest {
            stats.latency_us.push(arrived.elapsed().as_micros() as u32);
        }
    }
}

fn main() {
    let args = parse_args();
    let endpoint = EndpointSelection::from_environment().expect("an endpoint");
    let bridge = Rc::new(Bridge::start(ClientActivationPolicy::default(), endpoint));

    let started = futures::executor::block_on(bridge.start_session(
        Requested::Command(vec!["sh".to_owned(), "-c".to_owned(), args.command.clone()]),
        LaunchSite {
            working_directory: std::env::current_dir().ok(),
            rows: Some(args.rows),
            cols: Some(args.cols),
        },
    ))
    .expect("the bridge answered")
    .expect("the session started");
    let attached = futures::executor::block_on(bridge.attach(started.session_id.clone()))
        .expect("the bridge answered")
        .expect("attached");
    eprintln!(
        "session {} at {}x{}",
        started.session_id, args.rows, args.cols
    );

    Application::new().run(move |cx| {
        cx.activate(true);
        let cell_w = theme::TERMINAL_FONT_PX * 0.6 + 0.4;
        let cell_h = (theme::TERMINAL_FONT_PX * 1.25).round();
        let bounds = Bounds {
            origin: point(px(20.), px(40.)),
            size: size(
                px(cell_w * f32::from(args.cols) + 16.),
                px(cell_h * f32::from(args.rows) + 16.),
            ),
        };
        let coalesce = args.coalesce;
        let harness = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..WindowOptions::default()
                },
                |window, cx| {
                    window.activate_window();
                    cx.new(|cx| Harness::new(attached, coalesce, cx))
                },
            )
            .expect("a window");
        let view = harness.read(cx).map(|_| ()).ok();
        assert!(view.is_some(), "the window has a view");
        let duration = args.duration;
        let (rows, cols) = (args.rows, args.cols);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(duration).await;
            let line = harness
                .update(cx, |harness, _, _| {
                    let stats = harness.stats.borrow_mut();
                    let mut paint = stats.paint_us.clone();
                    let mut latency = stats.latency_us.clone();
                    format!(
                        "STATS size={rows}x{cols} seconds={} ticks={} frames={} bytes={} paints={} \
                         paint_us(p50/p95/max)={}/{}/{} arrival_to_paint_us(p50/p95)={}/{}",
                        duration.as_secs(),
                        stats.ticks,
                        stats.frames,
                        stats.bytes,
                        stats.paints,
                        percentile(&mut paint, 0.5),
                        percentile(&mut paint, 0.95),
                        percentile(&mut paint, 1.0),
                        percentile(&mut latency, 0.5),
                        percentile(&mut latency, 0.95),
                    )
                })
                .unwrap_or_default();
            println!("{line}");
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
