//! The alert the watcher opens when new SVGs would cut twice.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, AppContext, Bounds, Context, Div, FontWeight, MouseButton, Render, Window, WindowBounds, WindowKind,
    WindowOptions, canvas, div, linear_color_stop, linear_gradient, prelude::*, px, size,
};

use super::theme::*;
use super::{CloseWindow, Item, State, fix_one, review, settings};
use crate::analyze::Analysis;
use crate::config::{Config, Settings};

const WIDTH: f32 = 520.;
const MAX_ROWS: usize = 5;

struct Fixed {
    out: Result<PathBuf, String>,
    /// Sampled points of the original with no line left nearby (0 is good).
    missing: Option<usize>,
    /// Doubled length still in the fixed copy, if any.
    still_doubled: Option<f64>,
}

impl Fixed {
    fn verified(&self) -> bool {
        self.out.is_ok() && self.missing == Some(0) && self.still_doubled.is_none()
    }
}

enum Phase {
    Ask,
    Fixing,
    Done(Vec<Fixed>),
}

pub struct Alert {
    items: Vec<(PathBuf, Arc<Analysis>)>,
    phase: Phase,
    settings: Settings,
    rewarn_days: f64,
}

pub fn open(paths: Vec<PathBuf>, cx: &mut App) {
    let s = settings();
    let items: Vec<(PathBuf, Arc<Analysis>)> = paths
        .into_iter()
        .filter_map(|p| {
            let an = crate::analyse_file(&p, &s).ok()?;
            an.has_problems(&s).then(|| (p, Arc::new(an)))
        })
        .collect();
    if items.is_empty() {
        cx.quit();
        return;
    }
    let cfg = Config::load();
    if !cfg.alert_sound.is_empty() {
        let _ = std::process::Command::new("/usr/bin/afplay")
            .arg(format!("/System/Library/Sounds/{}.aiff", cfg.alert_sound))
            .spawn();
    }
    // A first guess; the card measures itself and the window snaps to fit.
    let rows = items.len().min(MAX_ROWS) as f32 + if items.len() > MAX_ROWS { 0.6 } else { 0. };
    let h = 2. + 100. + rows * 45. + 75.;
    let bounds = Bounds::centered(None, size(px(WIDTH), px(h)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            // Floats above everything else until dealt with.
            kind: WindowKind::PopUp,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            ..Default::default()
        },
        move |_, cx| {
            cx.new(|_| Alert {
                items,
                phase: Phase::Ask,
                settings: s,
                rewarn_days: cfg.rewarn_days,
            })
        },
    );
}

impl Alert {
    fn fix(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Ask) {
            return;
        }
        self.phase = Phase::Fixing;
        let items = self.items.clone();
        let s = self.settings;
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move {
                    items
                        .iter()
                        .map(|(p, an)| {
                            let out = fix_one(p, an);
                            let (missing, still_doubled) = match &out {
                                Ok(f) => match crate::analyse_file(f, &s) {
                                    Ok(fixed) => (
                                        Some(crate::verify::missing(an, &fixed, &s)),
                                        fixed.has_problems(&s).then_some(fixed.removed_mm),
                                    ),
                                    Err(_) => (None, None),
                                },
                                Err(_) => (None, None),
                            };
                            Fixed {
                                out,
                                missing,
                                still_doubled,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.phase = Phase::Done(results);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Hands over to the review window. Deferred: opening a window inside a
    /// click handler deadlocks, and the alert closes only once the review is
    /// up, or the app sees no windows and quits.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fixed = match &self.phase {
            Phase::Done(r) => r.iter().map(|f| Some(f.out.clone())).collect(),
            _ => vec![None; self.items.len()],
        };
        let items: Vec<Item> = self
            .items
            .iter()
            .zip(fixed)
            .map(|((p, an), fixed)| Item {
                path: p.clone(),
                state: State::Ready(an.clone()),
                fixed,
            })
            .collect();
        let alert = window.window_handle();
        cx.defer(move |cx| {
            review::open(vec![], items, cx);
            let _ = alert.update(cx, |_, window, _| window.remove_window());
        });
    }
}

fn days(d: f64) -> String {
    if d == 1.0 {
        "1 day".into()
    } else if d.fract() == 0.0 {
        format!("{d:.0} days")
    } else {
        format!("{d:.1} days")
    }
}

fn row(icon_name: &str, icon_color: Hsla, label: String, t: Theme) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(t.panel)
        .border_1()
        .border_color(t.border)
        .child(icon(icon_name, 14., icon_color))
        .child(
            text(label, 13., t.text)
                .font_weight(FontWeight::MEDIUM)
                .flex_1()
                .min_w_0()
                .truncate(),
        )
}

impl Render for Alert {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(window);
        let n = self.items.len();
        let total: f64 = self.items.iter().map(|(_, a)| a.removed_mm).sum();
        let done = match &self.phase {
            Phase::Done(r) => Some(r),
            _ => None,
        };
        let all_good = done.is_some_and(|r| r.iter().all(Fixed::verified));
        let accent = if all_good {
            t.ok
        } else if done.is_some() {
            t.double
        } else {
            t.laser
        };

        // Where the files turned up, if they share a folder.
        let mut dirs: Vec<&std::path::Path> = self.items.iter().filter_map(|(p, _)| p.parent()).collect();
        dirs.dedup();
        let place = match dirs.as_slice() {
            [one] => match one.file_name() {
                Some(n) => format!("in {}", n.to_string_lossy()),
                None => "on this Mac".into(),
            },
            _ => "in your watched folders".into(),
        };

        let (title, subtitle) = match done {
            None => (
                if n == 1 {
                    "A new SVG would cut twice".to_string()
                } else {
                    format!("{n} new SVGs would cut twice")
                },
                format!(
                    "Spotted {place}. The laser would trace {} of line a second time.",
                    length_words(total)
                ),
            ),
            Some(_) if all_good => (
                match n {
                    1 => "Fixed, every line cut once".to_string(),
                    2 => "Both fixed, every line cut once".to_string(),
                    _ => format!("All {n} fixed, every line cut once"),
                },
                "Fixed copies sit next to the originals. Send these to the laser instead.".to_string(),
            ),
            Some(_) => (
                "Some fixes need a look".to_string(),
                "Not every fixed copy passed its check. Open them before sending anything to the laser."
                    .to_string(),
            ),
        };

        let tile = if done.is_some() {
            div()
                .size(px(44.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(11.))
                .bg(accent.opacity(0.12))
                .border_1()
                .border_color(accent.opacity(0.25))
                .child(icon(
                    if all_good { "check-large" } else { "scan-search" },
                    20.,
                    accent,
                ))
        } else {
            div().flex_shrink_0().child(app_icon(48.))
        };

        let mut files = div().w_full().flex().flex_col().gap(px(6.)).px(px(24.));
        for (i, (p, an)) in self.items.iter().take(MAX_ROWS).enumerate() {
            let r = match done.and_then(|d| d.get(i)) {
                None => row("file", t.dim, name(p), t)
                    .child(mono(areas(an.areas.len()), 11., t.dim))
                    .child(mono(length(an.removed_mm), 11., t.double_text).font_weight(FontWeight::SEMIBOLD)),
                Some(f) => {
                    let (label, tag, color) = match (&f.out, f.missing, f.still_doubled) {
                        (Err(e), _, _) => (name(p), format!("couldn't save: {e}"), t.double_text),
                        (Ok(out), _, Some(mm)) => {
                            (name(out), format!("still {} twice", length(mm)), t.double_text)
                        }
                        (Ok(out), Some(0), None) => (name(out), "verified".into(), t.ok),
                        (Ok(out), Some(m), None) => (name(out), format!("{m} points lost"), t.double_text),
                        (Ok(out), None, None) => (name(out), "couldn't check".into(), t.double_text),
                    };
                    let ok = f.verified();
                    row(
                        if ok { "file-check" } else { "file" },
                        if ok { t.ok } else { t.double },
                        label,
                        t,
                    )
                    .child(mono(tag, 11., color).flex_shrink_0())
                }
            };
            files = files.child(r);
        }
        if n > MAX_ROWS {
            files = files.child(text(format!("and {} more", n - MAX_ROWS), 12., t.dim).pl(px(4.)));
        }

        let fix_label = match n {
            1 => "Fix it".to_string(),
            2 => "Fix both".to_string(),
            _ => format!("Fix all {n}"),
        };
        let actions = match &self.phase {
            Phase::Ask | Phase::Fixing => {
                let fixing = matches!(self.phase, Phase::Fixing);
                div()
                    .child(
                        text_button("ignore", format!("Ignore for {}", days(self.rewarn_days)), t)
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
                    .child(div().flex_1())
                    .child(
                        secondary_button("review", "Review", t)
                            .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                    )
                    .child(
                        primary_button(
                            "fix",
                            "wand-sparkles",
                            if fixing { "Fixing…".into() } else { fix_label },
                            t,
                        )
                        .when(fixing, |d| d.opacity(0.7))
                        .on_click(cx.listener(|this, _, _, cx| this.fix(cx))),
                    )
            }
            Phase::Done(r) => {
                let first = r.iter().find_map(|f| f.out.as_ref().ok().cloned());
                div()
                    .child(
                        text_button("show", "Show me", t)
                            .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                    )
                    .child(div().flex_1())
                    .when_some(first, |d, out| {
                        d.child(
                            base_secondary("finder", t)
                                .child(icon("folder-open", 14., t.muted))
                                .child("Show in Finder")
                                .on_click(move |_, _, cx| cx.reveal_path(&out)),
                        )
                    })
                    .child(
                        div()
                            .id("done")
                            .px(px(18.))
                            .py(px(7.))
                            .rounded(px(7.))
                            .bg(t.text)
                            .text_color(t.bg)
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .child("Done")
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
            }
        }
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(24.))
        .pt(px(18.))
        .pb(px(20.));

        let beam = div()
            .w_full()
            .h(px(2.))
            .flex()
            .child(div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(accent.opacity(0.), 0.),
                linear_color_stop(accent, 1.),
            )))
            .child(div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(accent, 0.),
                linear_color_stop(accent.opacity(0.), 1.),
            )));

        // macOS gives the borderless window its corners and shadow; the card
        // just fills it, and resizes the window to whatever height it needs.
        let fit = canvas(
            |bounds, window, cx| {
                let want = bounds.size.height;
                if (want - window.viewport_size().height).abs() > px(0.5) {
                    window.defer(cx, move |window, _| window.resize(size(px(WIDTH), want)));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let card = div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .child(fit)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(beam)
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(16.))
                    .px(px(24.))
                    .pt(px(22.))
                    .pb(px(18.))
                    .child(tile)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(text(title, 17., t.text).font_weight(FontWeight::SEMIBOLD))
                            .child(text(subtitle, 12.5, t.muted).line_height(px(18.))),
                    ),
            )
            .child(files)
            .child(actions);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .child(card)
    }
}

fn base_secondary(id: &'static str, t: Theme) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(7.))
        .rounded(px(7.))
        .bg(t.raised)
        .border_1()
        .border_color(t.border)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text)
        .cursor_pointer()
        .hover(move |s| s.bg(t.hover))
}

use gpui::Hsla;
