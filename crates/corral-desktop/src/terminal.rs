//! One session's terminal in the Desktop: the attachment, the replica, and
//! the geometry this client asks for, owned together.
//!
//! One Desktop-owned attachment per session (round 1, #5). PR3 froze the
//! multi-viewer semantics — shared geometry, last explicit resize wins — and
//! this entity does not enter them from inside one process: it has one
//! presentation host at a time, the main window's pane or a window of its
//! own, and a second `terminal.attach` for the same session is never made.

use std::rc::Rc;
use std::time::Duration;

use corral_protocol::terminal::{FrameKind, Sequence, TerminalFrame};
use futures::StreamExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, ClickEvent, Context, FocusHandle, Font, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Pixels, Render, Size, StatefulInteractiveElement, Styled, Window, div, px,
};
use qwertty_term_vt::snapshot::SnapshotWindow;

use crate::app::Paste;
use crate::bridge::{Attached, Outbound};
use crate::input::{self, KeyPress};
use crate::replica::{Applied, Geometry, Replica};
use crate::terminal_element::TerminalElement;
use crate::theme;

/// How long output is gathered before the window is asked to paint. The
/// spike's measured default: without it every delta of a storm notifies the
/// view, and the notify effects crowd the foreground executor.
const COALESCE: Duration = Duration::from_millis(4);

/// How long the cell grid must hold still before its size is sent. Coalesced
/// across window-resize churn, delivered promptly at rest (round 2, Q10).
const RESIZE_SETTLE: Duration = Duration::from_millis(100);

/// Where this terminal is currently shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Embedded,
    Standalone,
}

pub struct SessionTerminal {
    session_id: String,
    replica: Replica,
    /// The daemon's half. `None` once the channel ended or was detached.
    outbound: Option<Outbound>,
    /// Why the channel ended, once it has.
    ended: Option<String>,
    /// The daemon's last refusal on this channel, in its words.
    refusal: Option<String>,
    focus: FocusHandle,
    font: Font,
    font_px: Pixels,
    cell: Option<Size<Pixels>>,
    /// The cell grid the view last measured.
    grid: Option<Geometry>,
    /// The grid last sent as a `Resize`.
    sent: Option<Geometry>,
    resize_settling: bool,
    cache: Option<Rc<SnapshotWindow>>,
    dirty: bool,
    notify_scheduled: bool,
    host: Host,
}

impl SessionTerminal {
    pub fn new(session_id: String, attached: Attached, cx: &mut Context<Self>) -> Self {
        let Attached {
            promised,
            inbound,
            outbound,
            ..
        } = attached;
        let mut inbound = inbound;
        cx.spawn(async move |this, cx| {
            while let Some(frame) = inbound.next().await {
                if this
                    .update(cx, |this, cx| this.receive(&frame, cx))
                    .is_err()
                {
                    return;
                }
            }
            // The stream ended: the daemon closed the channel, or the socket
            // failed. Either way nothing more will arrive.
            let _ = this.update(cx, |this, cx| this.ended("the channel ended", cx));
        })
        .detach();

        Self {
            session_id,
            replica: Replica::new(promised),
            outbound: Some(outbound),
            ended: None,
            refusal: None,
            focus: cx.focus_handle(),
            font: theme::monospace(),
            font_px: px(theme::TERMINAL_FONT_PX),
            cell: None,
            grid: None,
            sent: None,
            resize_settling: false,
            cache: None,
            dirty: true,
            notify_scheduled: false,
            host: Host::Embedded,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    #[must_use]
    pub fn host(&self) -> Host {
        self.host
    }

    /// Moving between hosts is a local change: the new pane measures its own
    /// grid and may send one `Resize` (round 2, Q10).
    pub fn set_host(&mut self, host: Host, cx: &mut Context<Self>) {
        self.host = host;
        cx.notify();
    }

    /// Whether the channel is still open.
    #[must_use]
    pub fn is_attached(&self) -> bool {
        self.outbound.is_some()
    }

    /// Why the channel ended, once it has.
    pub fn ended_because(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    fn receive(&mut self, frame: &TerminalFrame, cx: &mut Context<Self>) {
        let applied = self.replica.apply(frame);
        self.act_on(applied, cx);
    }

    fn act_on(&mut self, applied: Applied, cx: &mut Context<Self>) {
        if applied.resync {
            self.send(FrameKind::ResyncRequest, Vec::new());
        }
        if let Some(refusal) = applied.refusal {
            self.refusal = Some(refusal);
            self.dirty = true;
            self.schedule_notify(cx);
        }
        if applied.redraw {
            self.dirty = true;
            self.schedule_notify(cx);
        }
    }

    fn ended(&mut self, reason: &str, cx: &mut Context<Self>) {
        if self.ended.is_none() {
            self.ended = Some(reason.to_owned());
        }
        self.outbound = None;
        cx.notify();
    }

    /// Close the channel. The run lives on: that is what detaching means.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        self.ended("Detached", cx);
    }

    fn send(&mut self, kind: FrameKind, payload: Vec<u8>) {
        let Some(outbound) = &self.outbound else {
            return;
        };
        let frame = TerminalFrame {
            kind,
            epoch: self.replica.epoch(),
            sequence: Sequence(0),
            payload,
        };
        outbound.send(frame);
    }

    /// Input for the session, in the frames the daemon accepts.
    fn input(&mut self, bytes: &[u8]) {
        if let Some(outbound) = &self.outbound {
            outbound.input(self.replica.epoch(), bytes);
        }
    }

    /// The accepted terminal representation of Ctrl-C, as input.
    pub fn interrupt(&mut self) {
        self.input(input::INTERRUPT);
    }

    fn schedule_notify(&mut self, cx: &mut Context<Self>) {
        if self.notify_scheduled {
            return;
        }
        self.notify_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COALESCE).await;
            let _ = this.update(cx, |this, cx| {
                this.notify_scheduled = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// The grid the pane holds, as the element measured it.
    ///
    /// Only a changed grid is ever sent, and only after it held still. A
    /// `Geometry` frame, a snapshot's size, an epoch, another viewer's resize
    /// are observations of daemon truth and never echo back (round 2, Q10).
    fn measured(&mut self, grid: Geometry, cx: &mut Context<Self>) {
        if self.grid == Some(grid) {
            return;
        }
        self.grid = Some(grid);
        if self.resize_settling {
            return;
        }
        self.resize_settling = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let _ = this.update(cx, |this, cx| this.settle_resize(cx));
        })
        .detach();
    }

    fn settle_resize(&mut self, cx: &mut Context<Self>) {
        self.resize_settling = false;
        let Some(grid) = self.grid else {
            return;
        };
        if self.sent == Some(grid) {
            return;
        }
        self.sent = Some(grid);
        self.send(FrameKind::Resize, grid.encode());
        let applied = self.replica.requested(grid);
        self.act_on(applied, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let press = KeyPress {
            key: keystroke.key.as_str(),
            typed: keystroke.key_char.as_deref(),
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            platform: keystroke.modifiers.platform,
        };
        if let Some(bytes) = input::encode(&press, self.replica.modes()) {
            self.input(&bytes);
            cx.stop_propagation();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bytes = input::paste(&text, self.replica.modes());
        self.input(&bytes);
    }

    fn take_focus(&mut self, _: &ClickEvent, window: &mut Window, _cx: &mut Context<Self>) {
        window.focus(&self.focus);
    }

    fn cell_size(&mut self, window: &Window) -> Size<Pixels> {
        if let Some(cell) = self.cell {
            return cell;
        }
        let cell = theme::cell_size(window, &self.font, self.font_px);
        self.cell = Some(cell);
        cell
    }
}

impl Render for SessionTerminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dirty {
            self.cache = self.replica.window().ok().map(Rc::new);
            self.dirty = false;
        }
        let cell = self.cell_size(window);
        let absence = self.replica.window().err().map(|absence| absence.line());
        let weak = cx.entity().downgrade();
        let element = TerminalElement {
            snapshot: self.cache.clone(),
            font: self.font.clone(),
            font_px: self.font_px,
            cell,
            known_grid: self.grid,
            on_measured: Rc::new(move |grid, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| this.measured(grid, cx));
            }),
        };
        let focused = self.focus.is_focused(window);

        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_click(cx.listener(Self::take_focus))
            .relative()
            .size_full()
            .bg(theme::terminal_bg())
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::border()
            })
            .child(element)
            .when_some(absence, |this, line| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::muted())
                        .child(line),
                )
            })
            .when_some(self.refusal.clone(), |this, refusal| {
                this.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .px_2()
                        .py_1()
                        .bg(theme::panel_bg())
                        .text_color(theme::danger())
                        .child(refusal),
                )
            })
    }
}
