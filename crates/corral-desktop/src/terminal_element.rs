//! The element that paints a replica's screen, cell by cell.
//!
//! The shape the spike measured (scenario 3): per row, consecutive narrow
//! cells are shaped as one line with style runs, and each wide cell is its own
//! segment placed at `col × cell_width`, so a fallback glyph whose advance is
//! not two cells cannot drift the rest of the row; backgrounds and the cursor
//! are quads. Paint p95 was 1.3 ms at 200×60 under a real display link,
//! inside the 8 ms budget with room.
//!
//! The element also measures. Its bounds are what the window gives it, and
//! the cell grid those bounds hold is the only thing the Desktop ever asks
//! the daemon to resize to (round 2, Q10): the measurement is reported to the
//! owner, which decides whether it changed.

use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, Size,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window, fill, point, px, relative, rgb,
    size,
};
use qwertty_term_vt::color::{Palette, Rgb};
use qwertty_term_vt::snapshot::{CellWidth, SnapshotColor, SnapshotUnderline, SnapshotWindow};

use crate::replica::Geometry;
use crate::theme;

/// What the owner learns from a paint: the grid the element's bounds hold.
pub type OnMeasured = Rc<dyn Fn(Geometry, &mut App)>;

pub struct TerminalElement {
    /// The screen to paint, or nothing while the replica has none. The
    /// element exists either way, because the grid must be measured before
    /// the first screen can exist under an old daemon (round 2, Q13).
    pub snapshot: Option<Rc<SnapshotWindow>>,
    pub font: Font,
    pub font_px: Pixels,
    pub cell: Size<Pixels>,
    /// The grid the owner already knows, so only a change is reported.
    pub known_grid: Option<Geometry>,
    pub on_measured: OnMeasured,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl TerminalElement {
    /// The grid these bounds hold. At least one cell each way: a pane too
    /// small for a cell is still a pane, and a zero-sized terminal is a
    /// request the daemon would refuse.
    fn grid_in(&self, bounds: &Bounds<Pixels>) -> Geometry {
        let rows = (bounds.size.height / self.cell.height).floor();
        let cols = (bounds.size.width / self.cell.width).floor();
        Geometry {
            rows: clamp_cells(rows),
            cols: clamp_cells(cols),
        }
    }
}

fn clamp_cells(count: f32) -> u16 {
    // Saturating on purpose: `as` from a float clamps to the target range and
    // maps NaN to zero, and the floor of a non-negative size is never NaN.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = count.max(1.0).min(f32::from(u16::MAX)) as u16;
    count.max(1)
}

impl Element for TerminalElement {
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
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // The whole pane: the grid follows the space the window gives, never
        // the other way round.
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _window: &mut Window,
        cx: &mut App,
    ) {
        let grid = self.grid_in(&bounds);
        if self.known_grid != Some(grid) {
            // Deferred: the owner is being rendered right now and cannot be
            // updated from inside its own frame.
            let on_measured = Rc::clone(&self.on_measured);
            cx.defer(move |cx| on_measured(grid, cx));
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(snapshot) = self.snapshot.clone() else {
            window.paint_quad(fill(bounds, theme::terminal_bg()));
            return;
        };
        let default_fg = snapshot.default_fg.map_or(theme::terminal_fg(), colour_of);
        let default_bg = snapshot.default_bg.map_or(theme::terminal_bg(), colour_of);
        window.paint_quad(fill(bounds, default_bg));

        // Clipped to the pane: a replica larger than the space it has — the
        // daemon has not reshaped yet — paints what fits, never over the
        // neighbours.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let painter = Painter {
                bounds,
                cell: self.cell,
                font: &self.font,
                font_px: self.font_px,
                palette: &snapshot.palette,
                default_fg,
                default_bg,
            };
            for (row_index, row) in snapshot.window.iter().enumerate() {
                let y = bounds.origin.y + self.cell.height * row_index as f32;
                if y > bounds.origin.y + bounds.size.height {
                    break;
                }
                painter.row(row_index, &row.cells, window, cx);
            }
            if snapshot.cursor.visible {
                let cursor = snapshot.cursor;
                window.paint_quad(fill(
                    Bounds {
                        origin: point(
                            bounds.origin.x + self.cell.width * cursor.col as f32,
                            bounds.origin.y + self.cell.height * cursor.row as f32,
                        ),
                        size: self.cell,
                    },
                    theme::cursor(),
                ));
            }
        });
    }
}

struct Painter<'a> {
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    font: &'a Font,
    font_px: Pixels,
    palette: &'a Palette,
    default_fg: Hsla,
    default_bg: Hsla,
}

impl Painter<'_> {
    fn row(
        &self,
        row_index: usize,
        cells: &[qwertty_term_vt::snapshot::SnapshotCell],
        window: &mut Window,
        cx: &mut App,
    ) {
        let y = self.bounds.origin.y + self.cell.height * row_index as f32;
        let mut col = 0_usize;
        while col < cells.len() {
            let start = col;
            let mut text = String::new();
            let mut runs: Vec<TextRun> = Vec::new();
            let wide = matches!(cells[col].width, CellWidth::Wide);
            loop {
                let cell = &cells[col];
                if matches!(cell.width, CellWidth::Spacer) {
                    col += 1;
                    break;
                }
                let style = &cell.style;
                let (mut fg, mut bg) = (
                    self.colour(style.fg, self.default_fg),
                    match style.bg {
                        SnapshotColor::Default => None,
                        other => Some(self.colour(other, self.default_bg)),
                    },
                );
                if style.inverse {
                    let behind = bg.unwrap_or(self.default_bg);
                    bg = Some(fg);
                    fg = behind;
                }
                if style.faint {
                    fg.a *= 0.6;
                }
                if let Some(bg) = bg {
                    let width = if matches!(cell.width, CellWidth::Wide) {
                        self.cell.width * 2.
                    } else {
                        self.cell.width
                    };
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(self.bounds.origin.x + self.cell.width * col as f32, y),
                            size: size(width, self.cell.height),
                        },
                        bg,
                    ));
                }
                let before = text.len();
                if !style.invisible {
                    text.push(cell.ch);
                    text.extend(cell.combining.iter());
                } else {
                    text.push(' ');
                }
                let len = text.len() - before;
                let font = Font {
                    weight: if style.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    style: if style.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    },
                    ..self.font.clone()
                };
                let underline =
                    (style.underline != SnapshotUnderline::None).then(|| UnderlineStyle {
                        thickness: px(1.),
                        color: Some(match style.underline_color {
                            SnapshotColor::Default => fg,
                            other => self.colour(other, fg),
                        }),
                        wavy: matches!(style.underline, SnapshotUnderline::Curly),
                    });
                let strikethrough = style.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(fg),
                });
                let mergeable = runs.last().is_some_and(|last: &TextRun| {
                    last.font == font
                        && last.color == fg
                        && last.underline == underline
                        && last.strikethrough == strikethrough
                });
                match runs.last_mut() {
                    Some(last) if mergeable => last.len += len,
                    _ => runs.push(TextRun {
                        len,
                        font,
                        color: fg,
                        background_color: None,
                        underline,
                        strikethrough,
                    }),
                }
                col += 1;
                if wide || col >= cells.len() || matches!(cells[col].width, CellWidth::Wide) {
                    break;
                }
            }
            if text.trim().is_empty() {
                continue;
            }
            let origin = point(self.bounds.origin.x + self.cell.width * start as f32, y);
            let line = window.text_system().shape_line(
                SharedString::from(text),
                self.font_px,
                &runs,
                None,
            );
            if wide {
                // A wide glyph from a fallback font may advance more than two
                // cells (the spike measured an emoji at 1.5 px over); it is
                // clipped to its slot rather than allowed onto the next cell.
                let slot = Bounds {
                    origin,
                    size: size(self.cell.width * 2., self.cell.height),
                };
                window.with_content_mask(Some(ContentMask { bounds: slot }), |window| {
                    let _ = line.paint(origin, self.cell.height, window, cx);
                });
            } else {
                let _ = line.paint(origin, self.cell.height, window, cx);
            }
        }
    }

    fn colour(&self, colour: SnapshotColor, default: Hsla) -> Hsla {
        match colour {
            SnapshotColor::Default => default,
            SnapshotColor::Palette(index) => colour_of(self.palette[usize::from(index)]),
            SnapshotColor::Rgb { r, g, b } => colour_of(Rgb { r, g, b }),
        }
    }
}

fn colour_of(rgb_: Rgb) -> Hsla {
    rgb((u32::from(rgb_.r) << 16) | (u32::from(rgb_.g) << 8) | u32::from(rgb_.b)).into()
}
