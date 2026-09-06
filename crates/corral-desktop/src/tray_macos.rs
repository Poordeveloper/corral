//! The status item on macOS: `tray-icon` for the item, `muda` for its menu,
//! both over the `objc2` AppKit bindings gpui already links (plan D3; grill
//! Q3). The mechanism composes with gpui's own `NSApplication` on the
//! evidence in `docs/references/2026-09-05-tray-probe.md`.
//!
//! Nothing here decides what the menu says: `TrayProjection::menu` does, and
//! this file maps its lines to native objects. Nothing here touches gpui
//! either: the menu handler runs on whichever thread AppKit chooses and only
//! forwards the clicked id; the Watch reads it on the foreground.

use futures::channel::mpsc::unbounded;
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{Clicks, Generations, MenuLine, StatusItem, TrayProjection};

/// The status item, for the life of the process. Dropping it removes the
/// item; the menu it shows is replaced whole on every changed projection,
/// and the generation the person may still have open stays alive until the
/// menu is next opened (`Generations`).
pub struct Tray {
    icon: TrayIcon,
    generations: Generations<Menu>,
}

impl Tray {
    /// Establish the status item — once per process, inside gpui's running
    /// loop: `tray-icon` requires the main thread's loop to be running on
    /// macOS, not merely created. The menu-event handler `muda` keeps is
    /// process-global and set once, so a second item would send its clicks
    /// nowhere; the Watch establishes one.
    pub fn establish() -> Result<(Self, Clicks), String> {
        let icon = TrayIconBuilder::new()
            .with_icon(glyph()?)
            .with_icon_as_template(true)
            .with_tooltip("Corral")
            .build()
            .map_err(|error| error.to_string())?;
        let (clicks, receiver) = unbounded::<String>();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = clicks.unbounded_send(event.id.0);
        }));
        let generations = Generations::new();
        let opener = generations.opener();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            // The press that opens the menu. tray-icon sends it before it
            // shows the menu, so the generation recorded is the one shown.
            if let TrayIconEvent::Click {
                button_state: MouseButtonState::Down,
                ..
            } = event
            {
                opener.opened();
            }
        }));
        Ok((Self { icon, generations }, receiver))
    }
}

impl StatusItem for Tray {
    fn show(&mut self, projection: &TrayProjection) -> Result<(), String> {
        let menu = Menu::new();
        for line in projection.menu() {
            match line {
                MenuLine::Note(text) => menu.append(&MenuItem::new(text, false, None)),
                MenuLine::Separator => menu.append(&PredefinedMenuItem::separator()),
                MenuLine::Item { action, text } => {
                    menu.append(&MenuItem::with_id(action.menu_id(), text, true, None))
                }
            }
            .map_err(|error| error.to_string())?;
        }
        // One generation: a menu whose ids carry their actions, swapped in
        // whole, and the badge with it (grill Q10). tray-icon drops the box
        // it is handed on the next swap; the clone shares the menu, and
        // `Generations` decides when the menu itself goes.
        let shown = self.generations.publish(menu).clone();
        self.icon.set_menu(Some(Box::new(shown)));
        self.icon.set_title(Some(title(projection)));
        Ok(())
    }
}

/// The status item's title: the badge, or the empty string — never `None`.
/// tray-icon 0.24.2 writes the native title only when handed one (tray-icon
/// #322), so a badge that has gone — resolved, or the daemon unreachable —
/// must be written as nothing, or the last count stays on the menu bar as
/// if current.
fn title(projection: &TrayProjection) -> String {
    projection.badge_text().unwrap_or_default()
}

/// The template glyph, built in code rather than shipped as an asset: a ring
/// open on the right. Black with alpha, so the menu bar tints it.
fn glyph() -> Result<Icon, String> {
    const SIDE: u32 = 36;
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    let centre = (SIDE as f32 - 1.0) / 2.0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let distance = (dx * dx + dy * dy).sqrt();
            let on_ring = (9.0..=15.0).contains(&distance) && !(dx > 6.0 && dy.abs() < 5.0);
            let alpha = if on_ring { 255 } else { 0 };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIDE, SIDE).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::{Badge, Current, Group};

    fn with_badge(badge: u32) -> TrayProjection {
        let group = |label| Group {
            label,
            total: 0,
            rows: vec![],
            overflow: 0,
        };
        TrayProjection::Current(Current {
            badge: Badge(badge),
            needs_you: group("Needs You"),
            ready: group("Ready"),
        })
    }

    /// A badge that has gone reaches the platform as an empty title: handed
    /// `None`, tray-icon 0.24.2 would leave the last count in place.
    #[test]
    fn an_absent_badge_is_written_as_an_empty_title() {
        assert_eq!(title(&with_badge(1)), "1");
        assert_eq!(title(&with_badge(0)), "");
        assert_eq!(
            title(&TrayProjection::Unreachable {
                line: "gone".to_owned()
            }),
            ""
        );
    }
}
