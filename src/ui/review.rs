//! The review window: files on the left, the sheet with numbered problem
//! areas in the middle, and an inspector listing the areas on the right.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, AppContext, Bounds, Context, Div, ExternalPaths, FocusHandle, FontWeight, ImageSource, MouseButton,
    Render, RenderImage, Stateful, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, point,
    prelude::*, px, relative, size,
};
use kurbo::Rect;

use super::theme::*;
use super::{
    CloseWindow, FixFile, Item, NextArea, PrevArea, ReviewWindow, State, fix_one, prompt_and_review,
    settings, to_render_image,
};
use crate::analyze::Analysis;
use crate::config::Settings;

const SIDEBAR: f32 = 272.;
const INSPECTOR: f32 = 264.;
const TITLEBAR: f32 = 52.;
const TOOLBAR: f32 = 56.;
const THUMB_W: f32 = 240.;
const THUMB_H: f32 = 136.;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Original,
    AfterFix,
}

/// Everything a rendered image depends on, so it's only redrawn when needed.
#[derive(Clone, PartialEq)]
struct Key {
    path: PathBuf,
    region: [u64; 4],
    w: u32,
    h: u32,
    dark: bool,
    focus: Option<usize>,
}

#[derive(Default)]
struct Slot {
    image: Option<(Key, Arc<RenderImage>)>,
    busy: bool,
}

#[derive(Clone, Copy)]
enum Which {
    Main,
    Thumb,
}

pub struct Review {
    files: Vec<Item>,
    selected: Option<usize>,
    /// Selected problem area of the selected file.
    area: Option<usize>,
    /// What the main preview shows; `None` is the whole sheet.
    zoom: Option<Rect>,
    view: View,
    /// Analyses of fixed copies, for "After fix". `None` while loading.
    fixed: HashMap<PathBuf, Option<Arc<Analysis>>>,
    main: Slot,
    thumb: Slot,
    settings: Settings,
    focus: FocusHandle,
}

/// Brings up the review window (reusing it if open) with these files added.
/// `ready` are files the alert has already analysed (and maybe fixed).
pub fn open(paths: Vec<PathBuf>, ready: Vec<Item>, cx: &mut App) {
    let mut ready = Some(ready);
    if let Some(handle) = cx.global::<ReviewWindow>().0 {
        let paths2 = paths.clone();
        let ok = handle
            .update(cx, |review, window, cx| {
                review.take(ready.take().unwrap_or_default(), cx);
                review.add(paths2, cx);
                window.activate_window();
            })
            .is_ok();
        if ok {
            return;
        }
    }
    let ready = ready.take().unwrap_or_default();
    let bounds = Bounds::centered(None, size(px(1200.), px(820.)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Laser Check".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(18.))),
                }),
                window_min_size: Some(size(px(980.), px(600.))),
                focus: true,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| {
                    let mut r = Review {
                        files: vec![],
                        selected: None,
                        area: None,
                        zoom: None,
                        view: View::Original,
                        fixed: HashMap::new(),
                        main: Slot::default(),
                        thumb: Slot::default(),
                        settings: settings(),
                        focus: cx.focus_handle(),
                    };
                    r.take(ready, cx);
                    r.add(paths, cx);
                    r
                });
                window.focus(&view.read(cx).focus.clone());
                view
            },
        )
        .ok();
    cx.set_global(ReviewWindow(handle));
}

fn is_fixed_name(p: &std::path::Path) -> bool {
    p.file_stem()
        .is_some_and(|s| s.to_string_lossy().ends_with("-fixed"))
}

impl Review {
    /// Adds items that were already analysed (from the alert).
    fn take(&mut self, items: Vec<Item>, cx: &mut Context<Self>) {
        for it in items {
            if let Some(pos) = self.files.iter().position(|f| f.path == it.path) {
                self.files[pos] = it;
            } else {
                self.files.push(it);
            }
        }
        self.link_fixed();
        self.pick_default();
        cx.notify();
    }

    fn add(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let s = self.settings;
        // Absolute paths, so the folder line and fixed-copy matching work.
        let mut paths: Vec<PathBuf> = crate::collect_svgs(paths)
            .into_iter()
            .map(|p| p.canonicalize().unwrap_or(p))
            .collect();
        // Originals first, so a name-fixed.svg can find its original.
        paths.sort_by_key(|p| is_fixed_name(p));
        for path in paths {
            if self
                .files
                .iter()
                .any(|f| f.path == path || f.fixed.as_ref().is_some_and(|x| x.as_ref() == Ok(&path)))
            {
                continue;
            }
            self.files.push(Item {
                path: path.clone(),
                state: State::Checking,
                fixed: None,
            });
            cx.spawn(async move |this, cx| {
                let p = path.clone();
                let res = cx
                    .background_spawn(async move { crate::analyse_file(&p, &s) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(it) = this.files.iter_mut().find(|f| f.path == path) {
                        it.state = match res {
                            Ok(an) => State::Ready(Arc::new(an)),
                            Err(e) => State::Failed(format!("{e:#}")),
                        };
                    }
                    this.pick_default();
                    cx.notify();
                });
            })
            .detach();
        }
        self.link_fixed();
        cx.notify();
    }

    /// Ties each original to its name-fixed.svg, whether that was added too,
    /// is sitting next to it on disk, or was just written.
    fn link_fixed(&mut self) {
        let listed: Vec<PathBuf> = self.files.iter().map(|f| f.path.clone()).collect();
        for it in &mut self.files {
            if it.fixed.is_some() || is_fixed_name(&it.path) {
                continue;
            }
            let fixed = crate::fix::fixed_path(&it.path);
            if listed.contains(&fixed) || fixed.exists() {
                it.fixed = Some(Ok(fixed));
            }
        }
    }

    /// Whether this item is someone else's fixed copy, shown nested under them.
    fn nested(&self, ix: usize) -> bool {
        let p = &self.files[ix].path;
        self.files
            .iter()
            .any(|o| o.path != *p && o.fixed.as_ref().is_some_and(|f| f.as_ref() == Ok(p)))
    }

    fn needs_fix(&self, it: &Item) -> bool {
        matches!(&it.state, State::Ready(a) if a.has_problems(&self.settings))
            && !matches!(it.fixed, Some(Ok(_)))
    }

    /// Rows for the sidebar: (to fix, fixed, clean).
    fn groups(&self) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let (mut to_fix, mut fixed, mut clean) = (vec![], vec![], vec![]);
        for (i, it) in self.files.iter().enumerate() {
            if self.nested(i) {
                continue;
            }
            match (&it.state, &it.fixed) {
                (_, Some(Ok(_))) => fixed.push(i),
                (State::Ready(a), _) if !a.has_problems(&self.settings) => clean.push(i),
                _ => to_fix.push(i),
            }
        }
        (to_fix, fixed, clean)
    }

    /// Selects the first file that needs fixing once everything is checked.
    fn pick_default(&mut self) {
        if self.selected.is_some()
            || self.files.is_empty()
            || self.files.iter().any(|f| matches!(f.state, State::Checking))
        {
            return;
        }
        let (to_fix, fixed, clean) = self.groups();
        let first = to_fix.first().or(fixed.first()).or(clean.first()).copied();
        if let Some(i) = first {
            self.select_file(i);
        }
    }

    fn select_file(&mut self, ix: usize) {
        self.selected = Some(ix);
        self.zoom = None;
        self.view = View::Original;
        self.area = self.analysis(ix).filter(|a| !a.areas.is_empty()).map(|_| 0);
    }

    fn analysis(&self, ix: usize) -> Option<Arc<Analysis>> {
        match &self.files.get(ix)?.state {
            State::Ready(a) => Some(a.clone()),
            _ => None,
        }
    }

    fn selected_analysis(&self) -> Option<Arc<Analysis>> {
        self.analysis(self.selected?)
    }

    fn fixed_path(&self) -> Option<PathBuf> {
        match &self.files.get(self.selected?)?.fixed {
            Some(Ok(p)) => Some(p.clone()),
            _ => None,
        }
    }

    fn select_area(&mut self, i: usize, zoom: bool, cx: &mut Context<Self>) {
        let Some(an) = self.selected_analysis() else {
            return;
        };
        let Some(area) = an.areas.get(i) else { return };
        self.area = Some(i);
        if zoom {
            self.zoom = Some(zoom_region(area.rect, an.doc.mm_per_unit));
        }
        cx.notify();
    }

    /// ↑/↓: move through areas. Follows along when zoomed in on one.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(an) = self.selected_analysis() else {
            return;
        };
        let n = an.areas.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self.area.map(|a| a as isize).unwrap_or(-1);
        let next = (cur + delta).rem_euclid(n) as usize;
        let zoomed = self.zoom.is_some();
        self.select_area(next, zoomed, cx);
    }

    fn zoom_by(&mut self, factor: f64, cx: &mut Context<Self>) {
        let Some(an) = self.selected_analysis() else {
            return;
        };
        let sheet = an.doc.view;
        let cur = self.zoom.unwrap_or(sheet);
        let next = Rect::from_center_size(cur.center(), (cur.width() * factor, cur.height() * factor));
        self.zoom = if next.width() >= sheet.width() && next.height() >= sheet.height() {
            None
        } else {
            Some(next)
        };
        cx.notify();
    }

    fn fix(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(it) = self.files.get_mut(ix) else { return };
        if let State::Ready(an) = &it.state
            && an.has_problems(&self.settings)
            && !matches!(it.fixed, Some(Ok(_)))
        {
            let res = fix_one(&it.path, an);
            if let Ok(p) = &res {
                self.fixed.remove(p);
            }
            it.fixed = Some(res);
            if self.selected == Some(ix) {
                self.view = View::AfterFix;
            }
        }
        cx.notify();
    }

    fn fix_all(&mut self, cx: &mut Context<Self>) {
        let todo: Vec<usize> = (0..self.files.len())
            .filter(|&i| self.needs_fix(&self.files[i]))
            .collect();
        for i in todo {
            self.fix(i, cx);
        }
    }

    /// The fixed copy's analysis, loading it in the background the first time.
    fn fixed_analysis(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Option<Arc<Analysis>> {
        if let Some(entry) = self.fixed.get(&path) {
            return entry.clone();
        }
        self.fixed.insert(path.clone(), None);
        let s = self.settings;
        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let res = cx
                .background_spawn(async move { crate::analyse_file(&p, &s) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(an) = res {
                    this.fixed.insert(path, Some(Arc::new(an)));
                }
                cx.notify();
            });
        })
        .detach();
        None
    }

    fn slot(&mut self, which: Which) -> &mut Slot {
        match which {
            Which::Main => &mut self.main,
            Which::Thumb => &mut self.thumb,
        }
    }

    /// The image for `key`, starting a render if it isn't there yet. While a
    /// render runs, the last image of the same file is shown.
    fn image(
        &mut self,
        which: Which,
        key: Key,
        an: Arc<Analysis>,
        region: Rect,
        focus: Option<Rect>,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let slot = self.slot(which);
        let current = slot
            .image
            .as_ref()
            .filter(|(k, _)| k.path == key.path)
            .map(|(_, i)| i.clone());
        if slot.image.as_ref().is_some_and(|(k, _)| *k == key) || slot.busy {
            return current;
        }
        slot.busy = true;
        cx.spawn(async move |this, cx| {
            let k = key.clone();
            let image = cx
                .background_spawn(async move {
                    let view = crate::render::View {
                        region,
                        width: k.w,
                        height: k.h,
                        focus,
                    };
                    to_render_image(crate::render::render(&an, &view, k.dark))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let slot = this.slot(which);
                slot.busy = false;
                if let Some((_, old)) = slot.image.replace((key, Arc::new(image))) {
                    cx.drop_image(old, None);
                }
                cx.notify();
            });
        })
        .detach();
        current
    }
}

/// Area to show when zoomed in on a problem: the cluster with some context,
/// never narrower than `min_mm` so tiny ones have something around them.
fn zoom_region(r: Rect, mm_per_unit: f64) -> Rect {
    zoom_region_min(r, mm_per_unit, 40.0)
}

fn zoom_region_min(r: Rect, mm_per_unit: f64, min_mm: f64) -> Rect {
    let min = min_mm / mm_per_unit;
    Rect::from_center_size(
        r.center(),
        ((r.width() * 1.4).max(min), (r.height() * 1.4).max(min)),
    )
}

/// Grows `region` to the aspect of a `w`x`h` canvas so that, once fitted, the
/// region sits centred inside the canvas minus the given insets (in px).
fn framed(region: Rect, w: f64, h: f64, insets: [f64; 4]) -> Rect {
    let [l, t, r, b] = insets;
    let (iw, ih) = ((w - l - r).max(10.0), (h - t - b).max(10.0));
    let s = (iw / region.width()).min(ih / region.height());
    let c = region.center();
    let x0 = c.x - (l + iw / 2.0) / s;
    let y0 = c.y - (t + ih / 2.0) / s;
    Rect::new(x0, y0, x0 + w / s, y0 + h / s)
}

fn key(path: PathBuf, region: Rect, w: f32, h: f32, scale: f32, dark: bool, focus: Option<usize>) -> Key {
    Key {
        path,
        region: [
            region.x0.to_bits(),
            region.y0.to_bits(),
            region.x1.to_bits(),
            region.y1.to_bits(),
        ],
        w: (w * scale).max(1.0) as u32,
        h: (h * scale).max(1.0) as u32,
        dark,
        focus,
    }
}

impl Render for Review {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(window);
        div()
            .id("root")
            .key_context("Review")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .on_action(cx.listener(|this, _: &NextArea, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PrevArea, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &FixFile, _, cx| {
                if let Some(i) = this.selected {
                    this.fix(i, cx)
                }
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add(paths.paths().to_vec(), cx)))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(t.panel))
            .child(self.titlebar(t, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.sidebar(t, cx))
                    .child(self.main(t, window, cx)),
            )
    }
}

impl Review {
    fn titlebar(&self, t: Theme, cx: &mut Context<Self>) -> Div {
        let (to_fix, fixed, clean) = self.groups();
        let total = to_fix.len() + fixed.len() + clean.len();
        let checking = self
            .files
            .iter()
            .filter(|f| matches!(f.state, State::Checking))
            .count();
        let bad = to_fix.iter().filter(|&&i| self.needs_fix(&self.files[i])).count();

        let pill = |label: String, fg: Hsla, bg: Hsla, dot: Hsla| {
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(bg)
                .child(div().size(px(6.)).rounded_full().bg(dot))
                .child(text(label, 12., fg).font_weight(FontWeight::MEDIUM))
        };
        let status = if total == 0 {
            None
        } else if checking > 0 {
            Some(pill(
                format!("Checking {} of {total}…", total - checking.min(total)),
                t.muted,
                t.raised,
                t.dim,
            ))
        } else if bad > 0 {
            let label = if total == 1 {
                "This file would cut twice".to_string()
            } else {
                format!("{bad} of {total} files would cut twice")
            };
            Some(pill(label, t.double_text, t.double.opacity(0.13), t.double))
        } else {
            let label = if total == 1 {
                "Every line cuts once".into()
            } else {
                format!("All {total} files cut once")
            };
            Some(pill(label, t.ok, t.ok.opacity(0.12), t.ok))
        };

        // Empty stretches of the bar move the window, like a normal title bar.
        let drag = |d: Div| {
            d.on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
        };

        div()
            .h(px(TITLEBAR))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .border_b_1()
            .border_color(t.border)
            // Room for the traffic lights, which macOS draws itself.
            .child(drag(div().w(px(52.)).h_full()))
            .child(drag(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(8.))
                    .h_full()
                    .child(app_icon(22.))
                    .child(text("Laser Check", 13., t.text).font_weight(FontWeight::SEMIBOLD)),
            ))
            .children(status)
            .child(drag(div().flex_1().h_full()))
            .child(
                div()
                    .id("add")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.raised))
                    .child(icon("plus", 14., t.text))
                    .child(text("Add files", 12.5, t.text).font_weight(FontWeight::MEDIUM))
                    .on_click(cx.listener(|_, _, _, cx| prompt_and_review(cx))),
            )
            .when(bad > 1, |d| {
                d.child(
                    primary_button("fix-all", "wand-sparkles", format!("Fix all {bad}"), t)
                        .px(px(12.))
                        .py(px(6.))
                        .shadow(vec![])
                        .on_click(cx.listener(|this, _, _, cx| this.fix_all(cx))),
                )
            })
    }

    fn sidebar(&self, t: Theme, cx: &mut Context<Self>) -> Div {
        let (to_fix, fixed, clean) = self.groups();
        let section = |label: String| {
            div()
                .w_full()
                .pt(px(10.))
                .pb(px(6.))
                .px(px(8.))
                .child(mono(label, 10.5, t.dim).font_weight(FontWeight::SEMIBOLD))
        };
        let mut list = div()
            .id("files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.));
        for (label, rows) in [("TO FIX", &to_fix), ("FIXED", &fixed), ("CLEAN", &clean)] {
            if rows.is_empty() {
                continue;
            }
            list = list.child(section(format!("{label} · {}", rows.len())));
            for &i in rows {
                list = list.child(self.file_row(i, t, cx));
            }
        }

        div()
            .w(px(SIDEBAR))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(14.))
            .bg(t.panel)
            .border_r_1()
            .border_color(t.border)
            .child(list)
            .child(
                div()
                    .id("drop-hint")
                    .mt(px(8.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .p(px(14.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.outline)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.raised))
                    .child(icon("file-plus", 14., t.dim))
                    .child(text("Drop SVGs or folders anywhere", 12., t.dim))
                    .on_click(cx.listener(|_, _, _, cx| prompt_and_review(cx))),
            )
    }

    fn file_row(&self, i: usize, t: Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let it = &self.files[i];
        let selected = self.selected == Some(i);
        let row = div()
            .id(("file", i))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(10.))
            .py(px(9.))
            .rounded(px(8.))
            .border_1()
            .border_color(if selected {
                t.outline
            } else {
                gpui::transparent_black()
            })
            .when(selected, |d| d.bg(t.raised))
            .when(!selected, |d| d.hover(move |s| s.bg(t.raised.opacity(0.5))))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_file(i);
                window.focus(&this.focus);
                cx.notify();
            }));
        let weight = if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        };
        let file_name = text(name(&it.path), 13., t.text)
            .font_weight(weight)
            .truncate()
            .min_w_0();

        if let Some(Ok(fixed)) = &it.fixed {
            let stem = it
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let fixed_name = name(fixed);
            let short = fixed_name
                .strip_prefix(&stem)
                .map(str::to_string)
                .unwrap_or(fixed_name);
            let removed = self
                .analysis(i)
                .map(|a| format!(" · {} removed", length(a.removed_mm)))
                .unwrap_or_default();
            return row
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("check", 12., t.ok))
                        .child(file_name.text_color(if selected { t.text } else { t.muted })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pl(px(16.))
                        .child(icon("corner-down-right", 12., t.ok))
                        .child(mono(format!("{short}{removed}"), 11., t.ok).truncate()),
                );
        }

        let (dot, meta, meta_color) = match (&it.state, &it.fixed) {
            (_, Some(Err(e))) => (t.double, format!("fix failed: {e}"), t.double_text),
            (State::Checking, _) => (t.dim, "checking…".into(), t.dim),
            (State::Failed(_), _) => (t.double, "couldn't read this file".into(), t.double_text),
            (State::Ready(a), _) if a.has_problems(&self.settings) => (
                t.double,
                format!("{} twice · {}", length(a.removed_mm), areas(a.areas.len())),
                if selected { t.double_text } else { t.muted },
            ),
            (State::Ready(_), _) => (t.ok, "cuts once".into(), t.muted),
        };
        row.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(dot))
                .child(file_name),
        )
        .child(div().pl(px(15.)).child(mono(meta, 11., meta_color).truncate()))
    }

    fn main(&mut self, t: Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(ix) = self.selected else {
            if self.files.is_empty() {
                return pane.child(empty_state(t, cx));
            }
            return pane
                .items_center()
                .justify_center()
                .child(text("Checking…", 13., t.muted));
        };
        let path = self.files[ix].path.clone();
        let an = match &self.files[ix].state {
            State::Ready(a) => a.clone(),
            State::Checking => {
                return pane
                    .items_center()
                    .justify_center()
                    .child(text("Checking…", 13., t.muted));
            }
            State::Failed(e) => {
                return pane.items_center().justify_center().child(text(
                    format!("Couldn't read this file: {e}"),
                    13.,
                    t.double_text,
                ));
            }
        };
        let bad = an.has_problems(&self.settings);
        let fixed = self.fixed_path();
        let needs_fix = bad && fixed.is_none();
        if fixed.is_none() {
            self.view = View::Original;
        }

        // Toolbar.
        let toggle = (bad || fixed.is_some()).then(|| {
            let seg = |id: &'static str, label: &'static str, active: bool, enabled: bool| {
                div()
                    .id(id)
                    .px(px(12.))
                    .py(px(5.))
                    .rounded(px(6.))
                    .when(active, |d| d.bg(t.hover))
                    .when(enabled && !active, |d| {
                        d.cursor_pointer().hover(move |s| s.text_color(t.text))
                    })
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if active {
                        t.text
                    } else if enabled {
                        t.muted
                    } else {
                        t.dim.opacity(0.6)
                    })
                    .child(label)
            };
            let after_enabled = fixed.is_some();
            div()
                .flex()
                .flex_shrink_0()
                .p(px(3.))
                .rounded(px(8.))
                .bg(t.panel)
                .border_1()
                .border_color(t.border)
                .child(
                    seg("view-original", "Original", self.view == View::Original, true).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.view = View::Original;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    seg(
                        "view-after",
                        "After fix",
                        self.view == View::AfterFix,
                        after_enabled,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if after_enabled {
                            this.view = View::AfterFix;
                            cx.notify();
                        }
                    })),
                )
        });
        let reveal_target = match (&fixed, self.view) {
            (Some(f), View::AfterFix) => f.clone(),
            _ => path.clone(),
        };
        let toolbar = div()
            .h(px(TOOLBAR))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(12.))
            .pl(px(20.))
            .pr(px(16.))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        text(name(&path), 15., t.text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate(),
                    )
                    .child(mono(folder(&path), 11., t.dim).truncate()),
            )
            .children(toggle)
            .child(
                icon_button("reveal", "folder-search", 16., 8., t)
                    .rounded(px(7.))
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal_target)),
            )
            .when(needs_fix, |d| {
                d.child(
                    primary_button("fix", "wand-sparkles", "Fix this file", t)
                        .child(mono("⌘↩", 10.5, t.on_laser.opacity(0.6)))
                        .on_click(cx.listener(move |this, _, _, cx| this.fix(ix, cx))),
                )
            });

        // Workspace: preview and, for files with problems, the inspector.
        let viewport = window.viewport_size();
        let scale = window.scale_factor();
        let dark = is_dark(window);
        let inspector_w = if bad { INSPECTOR } else { 0. };
        let pw = (f32::from(viewport.width) - SIDEBAR - inspector_w).max(200.);
        let ph = (f32::from(viewport.height) - TITLEBAR - TOOLBAR).max(200.);
        let preview = self.preview(ix, an.clone(), pw, ph, scale, dark, t, cx);

        let mut work = div().flex_1().min_h_0().flex().child(preview);
        if bad {
            work = work.child(self.inspector(ix, an, fixed, scale, dark, t, cx));
        }
        pane.child(toolbar).child(work)
    }

    #[allow(clippy::too_many_arguments)]
    fn preview(
        &mut self,
        ix: usize,
        original: Arc<Analysis>,
        w: f32,
        h: f32,
        scale: f32,
        dark: bool,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let path = self.files[ix].path.clone();
        let after = self.view == View::AfterFix;
        let (shown_path, shown) = match (after, self.fixed_path()) {
            (true, Some(fp)) => (fp.clone(), self.fixed_analysis(fp, cx)),
            _ => (path, Some(original.clone())),
        };
        let bad = original.has_problems(&self.settings);
        let insets = [16.0, 64.0, 16.0, 64.0];
        let sheet = original.doc.view;
        let region = framed(self.zoom.unwrap_or(sheet), w as f64, h as f64, insets);
        let focus_ix = if self.zoom.is_some() && !after {
            self.area
        } else {
            None
        };
        let focus_rect = focus_ix.and_then(|i| original.areas.get(i)).map(|a| a.rect);

        let mut pane = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(t.canvas)
            .overflow_hidden();
        match shown {
            Some(an) => {
                let k = key(shown_path, region, w, h, scale, dark, focus_ix);
                if let Some(image) = self.image(Which::Main, k, an.clone(), region, focus_rect, cx) {
                    pane = pane.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .child(img(ImageSource::Render(image)).w(px(w)).h(px(h))),
                    );
                }
                if after {
                    let (label, color) = if an.has_problems(&self.settings) {
                        (
                            format!("The fixed copy still cuts {} twice", length(an.removed_mm)),
                            t.double_text,
                        )
                    } else {
                        ("After fix: every line is cut once".to_string(), t.ok)
                    };
                    pane = pane.child(banner("check", label, color, t));
                }
            }
            None => {
                pane = pane
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text("Drawing…", 13., t.muted));
            }
        }

        if !after && bad {
            // Markers sit over the image at the same scale it was drawn at.
            let k = w as f64 / region.width();
            let zoomed = self.zoom.is_some();
            let to_px = |p: kurbo::Point| ((p.x - region.x0) * k, (p.y - region.y0) * k);
            let mut markers: Vec<(usize, f64, f64)> = original
                .areas
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    // Whole sheet: on the area. Zoomed in: just off its top-left
                    // corner, so the marker doesn't hide the line it points at.
                    let (x, y) = if zoomed {
                        let (x, y) = to_px(a.rect.origin());
                        (x - 16.0, y - 16.0)
                    } else {
                        to_px(a.rect.center())
                    };
                    (i, x, y)
                })
                .filter(|&(_, x, y)| x >= 0.0 && y >= 0.0 && x <= w as f64 && y <= h as f64)
                .collect();
            // Selected one last, so it's drawn on top.
            markers.sort_by_key(|&(i, _, _)| Some(i) == self.area);
            for (i, x, y) in markers {
                pane = pane.child(self.marker(i, x as f32, y as f32, t, cx));
            }
            // Only nag when the doubled lines really are too small to make out.
            let largest = original
                .areas
                .iter()
                .map(|a| a.rect.width().max(a.rect.height()) * k)
                .fold(0.0, f64::max);
            if largest < 24.0 {
                pane = pane.child(banner(
                    "scan-search",
                    "Too small to see at this zoom. Click a number, or press ↓ to step through.".into(),
                    t.marker_text,
                    t,
                ));
            }
            pane = pane.child(legend(t));
        } else if !after {
            pane = pane.child(banner("check", "Every line here is cut once.".into(), t.ok, t));
        }
        pane.child(self.zoom_controls(t, cx))
    }

    fn marker(&self, i: usize, x: f32, y: f32, t: Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.area == Some(i);
        let d = if selected { 26. } else { 22. };
        let m = div()
            .id(("marker", i))
            .absolute()
            .left(px(x - d / 2.))
            .top(px(y - d / 2.))
            .size(px(d))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.select_area(i, true, cx)));
        if selected {
            border_width(m, 2.)
                .border_color(hex(0xFFE0B8))
                .bg(t.laser)
                .shadow(shadow(t.laser.opacity(0.6), 0., 18.))
                .child(mono(format!("{}", i + 1), 11., t.on_laser).font_weight(FontWeight::BOLD))
        } else {
            border_width(m, 1.5)
                .border_color(t.double)
                .bg(t.canvas.opacity(0.8))
                .shadow(shadow(t.double.opacity(0.4), 0., 10.))
                .hover(move |s| s.bg(t.raised))
                .child(mono(format!("{}", i + 1), 10., t.marker_text).font_weight(FontWeight::BOLD))
        }
    }

    fn zoom_controls(&self, t: Theme, cx: &mut Context<Self>) -> Div {
        let b = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .px(px(12.))
                .py(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(t.raised))
                .child(icon(name, 14., t.muted))
        };
        div()
            .absolute()
            .right(px(16.))
            .bottom(px(16.))
            .flex()
            .items_center()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(t.panel.opacity(0.9))
            .border_1()
            .border_color(t.border)
            .child(b("zoom-out", "minus").on_click(cx.listener(|this, _, _, cx| this.zoom_by(2.0, cx))))
            .child(b("zoom-fit", "scan").on_click(cx.listener(|this, _, _, cx| {
                this.zoom = None;
                cx.notify();
            })))
            .child(b("zoom-in", "plus").on_click(cx.listener(|this, _, _, cx| this.zoom_by(0.5, cx))))
    }

    #[allow(clippy::too_many_arguments)]
    fn inspector(
        &mut self,
        ix: usize,
        an: Arc<Analysis>,
        fixed: Option<PathBuf>,
        scale: f32,
        dark: bool,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let path = self.files[ix].path.clone();
        let n = an.areas.len();
        let header = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pt(px(16.))
            .px(px(16.))
            .pb(px(12.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(mono(length(an.removed_mm), 22., t.double_text).font_weight(FontWeight::SEMIBOLD))
                    .child(mono(format!("in {}", areas(n)), 11.5, t.muted)),
            )
            .child(text("The laser would go over this much line twice.", 12., t.muted).line_height(px(17.)));

        // The selected area, zoomed in.
        let selected = self.area.filter(|&i| i < n);
        let area_block = selected.map(|i| {
            let area = an.areas[i];
            // Tighter than the main zoom, so the doubled line is visible this small.
            let region = framed(
                zoom_region_min(area.rect, an.doc.mm_per_unit, 12.0),
                THUMB_W as f64,
                THUMB_H as f64,
                [8.0, 8.0, 8.0, 8.0],
            );
            let k = key(path.clone(), region, THUMB_W, THUMB_H, scale, dark, Some(i));
            let image = self.image(Which::Thumb, k, an.clone(), region, Some(area.rect), cx);
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(12.))
                .pb(px(12.))
                .child(
                    div()
                        .relative()
                        .w(px(THUMB_W))
                        .h(px(THUMB_H))
                        .rounded(px(8.))
                        .overflow_hidden()
                        .bg(t.canvas)
                        .border_1()
                        .border_color(t.laser.opacity(0.4))
                        .children(image.map(|im| img(ImageSource::Render(im)).w(px(THUMB_W)).h(px(THUMB_H))))
                        .child(
                            div()
                                .absolute()
                                .top(px(8.))
                                .left(px(8.))
                                .px(px(8.))
                                .py(px(3.))
                                .rounded_full()
                                .bg(t.laser)
                                .child(
                                    mono(format!("AREA {}", i + 1), 10., t.on_laser)
                                        .font_weight(FontWeight::BOLD),
                                ),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            icon_button("prev-area", "chevron-up", 14., 6., t)
                                .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                        )
                        .child(
                            icon_button("next-area", "chevron-down", 14., 6., t)
                                .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                        )
                        .child(div().flex_1())
                        .child(mono(
                            format!("{} of {n} · {}", i + 1, length(area.mm)),
                            11.,
                            t.text,
                        )),
                )
        });

        let max = an.areas.iter().map(|a| a.mm).fold(0.0, f64::max).max(1e-9);
        let list = div()
            .id("areas")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .children(an.areas.iter().enumerate().map(|(i, a)| {
                let sel = selected == Some(i);
                let num = div()
                    .size(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(sel, |d| d.bg(t.laser))
                    .when(!sel, |d| d.border_1().border_color(t.double.opacity(0.5)))
                    .child(
                        mono(
                            format!("{}", i + 1),
                            10.,
                            if sel { t.on_laser } else { t.marker_text },
                        )
                        .font_weight(FontWeight::BOLD),
                    );
                let bar = div().flex_1().h(px(4.)).rounded(px(2.)).bg(t.raised).child(
                    div()
                        .h_full()
                        .w(relative((a.mm / max).clamp(0.02, 1.0) as f32))
                        .rounded(px(2.))
                        .bg(if sel { t.laser } else { t.double.opacity(0.6) }),
                );
                div()
                    .id(("area", i))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(t.raised))
                    .when(!sel, |d| d.hover(move |s| s.bg(t.raised.opacity(0.5))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let zoomed = this.zoom.is_some();
                        this.select_area(i, zoomed, cx)
                    }))
                    .child(num)
                    .child(bar)
                    .child(div().w(px(52.)).flex().justify_end().child(mono(
                        length(a.mm),
                        11.,
                        if sel { t.text } else { t.muted },
                    )))
            }));

        let note = match &fixed {
            Some(f) => format!("Saved {} alongside. Send that one to the laser.", name(f)),
            None => format!(
                "Fixing keeps the first copy of each line and saves {} alongside.",
                name(&crate::fix::fixed_path(&path))
            ),
        };
        let footer = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("shield-check", 13., t.ok))
                    .child(text("Original stays untouched", 12., t.text).font_weight(FontWeight::SEMIBOLD)),
            )
            .child(text(note, 11.5, t.muted).line_height(px(17.)));

        div()
            .w(px(INSPECTOR))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(t.panel)
            .border_l_1()
            .border_color(t.border)
            .child(header)
            .children(area_block)
            .child(div().w_full().h(px(1.)).bg(t.border))
            .child(list)
            .child(footer)
    }
}

/// A note pinned to the top left of the preview.
fn banner(icon_name: &str, label: String, color: Hsla, t: Theme) -> Div {
    let _ = t;
    div()
        .absolute()
        .top(px(16.))
        .left(px(16.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(7.))
        .rounded(px(8.))
        .bg(color.opacity(0.1))
        .border_1()
        .border_color(color.opacity(0.25))
        .child(icon(icon_name, 14., color))
        .child(text(label, 12., color))
}

fn legend(t: Theme) -> Div {
    let key = |swatch: Div, label: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(swatch)
            .child(text(label, 12., t.muted))
    };
    div()
        .absolute()
        .left(px(16.))
        .bottom(px(16.))
        .flex()
        .items_center()
        .gap(px(16.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(t.panel.opacity(0.9))
        .border_1()
        .border_color(t.border)
        .child(key(div().w(px(18.)).h(px(1.)).bg(hex(0x707996)), "Cut once"))
        .child(key(
            div()
                .w(px(18.))
                .h(px(3.))
                .rounded(px(2.))
                .bg(t.double)
                .shadow(shadow(t.double.opacity(0.67), 0., 6.)),
            "Cut twice",
        ))
}

fn empty_state(t: Theme, cx: &mut Context<Review>) -> Div {
    div().flex_1().p(px(24.)).flex().child(
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(12.))
            .items_center()
            .justify_center()
            .rounded(px(12.))
            .border_2()
            .border_dashed()
            .border_color(t.outline)
            .bg(t.canvas)
            .child(icon("file-plus", 28., t.dim))
            .child(text("Drop SVG files or folders here", 17., t.text).font_weight(FontWeight::SEMIBOLD))
            .child(text(
                "Each file is checked for lines the laser would cut more than once.",
                12.5,
                t.muted,
            ))
            .child(
                secondary_button("choose", "Choose files…", t)
                    .mt(px(4.))
                    .on_click(cx.listener(|_, _, _, cx| prompt_and_review(cx))),
            ),
    )
}

use gpui::Hsla;
