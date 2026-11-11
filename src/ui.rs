//! The windows: a review window you can drop files and folders onto, and the
//! alert the watcher pops up.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Context, Div, ExternalPaths, FontWeight, Global, Hsla, KeyBinding,
    Menu, MenuItem, ObjectFit, PathPromptOptions, Render, RenderImage, SharedString, Stateful,
    TitlebarOptions, Window, WindowAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    actions, div, img, prelude::*, px, rgb, size,
};
use kurbo::Rect;

use crate::analyze::Analysis;
use crate::config::{Config, Settings};

actions!(laser_check, [Quit, CloseWindow, OpenFiles]);

pub enum Start {
    Review(Vec<PathBuf>),
    Alert(Vec<PathBuf>),
}

struct ReviewWindow(Option<WindowHandle<Review>>);
impl Global for ReviewWindow {}

pub fn run(start: Start) {
    let app = Application::new();
    // Files opened from Finder ("Open With", or dropped on the Dock icon).
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| {
        let _ = tx.unbounded_send(urls);
    });
    app.run(move |cx: &mut App| {
        cx.set_global(ReviewWindow(None));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenFiles, cx| prompt_and_review(cx));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-o", OpenFiles, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "Laser Check".into(),
                items: vec![MenuItem::action("Quit Laser Check", Quit)],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open…", OpenFiles),
                    MenuItem::action("Close Window", CloseWindow),
                ],
            },
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(urls) = rx.next().await {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| file_url_to_path(u)).collect();
                let _ = cx.update(|cx| open_review(paths, vec![], cx));
            }
        })
        .detach();

        match start {
            Start::Review(paths) => open_review(paths, vec![], cx),
            Start::Alert(paths) => open_alert(paths, cx),
        }
        cx.activate(true);
    });
}

fn file_url_to_path(u: &str) -> Option<PathBuf> {
    let rest = u.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

// ---------------------------------------------------------------- colours

#[derive(Clone, Copy)]
struct Theme {
    bg: Hsla,
    panel: Hsla,
    text: Hsla,
    muted: Hsla,
    border: Hsla,
    red: Hsla,
    red_soft: Hsla,
    green: Hsla,
    button: Hsla,
    button_hover: Hsla,
}

fn theme(window: &Window) -> Theme {
    let dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    if dark {
        Theme {
            bg: rgb(0x1e1e20).into(),
            panel: rgb(0x26262a).into(),
            text: rgb(0xececf0).into(),
            muted: rgb(0x9a9aa2).into(),
            border: rgb(0x3a3a40).into(),
            red: rgb(0xff453a).into(),
            red_soft: rgb(0x4a1f1f).into(),
            green: rgb(0x32d74b).into(),
            button: rgb(0x3a3a40).into(),
            button_hover: rgb(0x4a4a52).into(),
        }
    } else {
        Theme {
            bg: rgb(0xffffff).into(),
            panel: rgb(0xf4f4f6).into(),
            text: rgb(0x1d1d1f).into(),
            muted: rgb(0x6e6e73).into(),
            border: rgb(0xdcdce0).into(),
            red: rgb(0xd70015).into(),
            red_soft: rgb(0xffe5e5).into(),
            green: rgb(0x248a3d).into(),
            button: rgb(0xe8e8ec).into(),
            button_hover: rgb(0xdadade).into(),
        }
    }
}

fn is_dark(window: &Window) -> bool {
    matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, t: Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .px_3()
        .py_1()
        .rounded_md()
        .bg(t.button)
        .text_color(t.text)
        .text_sm()
        .cursor_pointer()
        .hover(move |s| s.bg(t.button_hover))
        .child(label.into())
}

fn primary(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, t: Theme) -> Stateful<Div> {
    let hover = t.red.opacity(0.85);
    button(id, label, t)
        .bg(t.red)
        .text_color(gpui::white())
        .font_weight(FontWeight::SEMIBOLD)
        .hover(move |s| s.bg(hover))
}

pub fn areas(n: usize) -> String {
    if n == 1 {
        "1 area".into()
    } else {
        format!("{n} areas")
    }
}

pub fn metres(mm: f64) -> String {
    if mm >= 1000.0 {
        format!("{:.2} m", mm / 1000.0)
    } else {
        format!("{mm:.0} mm")
    }
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn settings() -> Settings {
    Config::load().settings()
}

// ---------------------------------------------------------------- review window

pub enum State {
    Checking,
    Ready(Arc<Analysis>),
    Failed(String),
}

pub struct Item {
    pub path: PathBuf,
    pub state: State,
    pub fixed: Option<Result<PathBuf, String>>,
}

#[derive(Clone, PartialEq)]
struct Key {
    path: PathBuf,
    area: Option<usize>,
    w: u32,
    h: u32,
    dark: bool,
}

pub struct Review {
    files: Vec<Item>,
    selected: Option<usize>,
    area: Option<usize>,
    preview: Option<(Key, Arc<RenderImage>)>,
    rendering: bool,
    settings: Settings,
}

const SIDEBAR: f32 = 300.0;
const HEADER: f32 = 56.0;
const TOOLBAR: f32 = 48.0;

fn prompt_and_review(cx: &mut App) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: true,
        multiple: true,
        prompt: Some("Check".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            let _ = cx.update(|cx| open_review(paths, vec![], cx));
        }
    })
    .detach();
}

/// Brings up the review window (reusing it if open) with these files added.
pub fn open_review(paths: Vec<PathBuf>, ready: Vec<Item>, cx: &mut App) {
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
                    ..Default::default()
                }),
                window_min_size: Some(size(px(760.), px(480.))),
                focus: true,
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| {
                    let mut r = Review {
                        files: vec![],
                        selected: None,
                        area: None,
                        preview: None,
                        rendering: false,
                        settings: settings(),
                    };
                    r.take(ready, cx);
                    r.add(paths, cx);
                    r
                })
            },
        )
        .ok();
    cx.set_global(ReviewWindow(handle));
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
        self.pick_default();
        cx.notify();
    }

    fn add(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let s = self.settings;
        for path in crate::collect_svgs(paths) {
            if self.files.iter().any(|f| f.path == path) {
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
        cx.notify();
    }

    /// Selects the first file with problems once there is one, else the first file.
    fn pick_default(&mut self) {
        if self.selected.is_some() {
            return;
        }
        // Wait for every file so the pick doesn't depend on which finished first.
        if self.files.is_empty() || self.files.iter().any(|f| matches!(f.state, State::Checking)) {
            return;
        }
        let s = self.settings;
        let bad = self
            .files
            .iter()
            .position(|f| matches!(&f.state, State::Ready(a) if a.has_problems(&s)));
        self.selected = bad.or(Some(0));
        self.area = None;
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.area = None;
        cx.notify();
    }

    fn step_area(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(an) = self.selected_analysis() else {
            return;
        };
        let n = an.areas.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self
            .area
            .map(|a| a as isize)
            .unwrap_or(if delta > 0 { -1 } else { n });
        self.area = Some((cur + delta).rem_euclid(n) as usize);
        cx.notify();
    }

    fn selected_analysis(&self) -> Option<Arc<Analysis>> {
        match &self.files.get(self.selected?)?.state {
            State::Ready(a) => Some(a.clone()),
            _ => None,
        }
    }

    fn fix(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(it) = self.files.get_mut(ix) else { return };
        if let State::Ready(an) = &it.state {
            it.fixed = Some(fix_one(&it.path, an));
        }
        cx.notify();
    }

    fn fix_all(&mut self, cx: &mut Context<Self>) {
        let s = self.settings;
        for it in &mut self.files {
            if let State::Ready(an) = &it.state
                && an.has_problems(&s)
                && it.fixed.is_none()
            {
                it.fixed = Some(fix_one(&it.path, an));
            }
        }
        cx.notify();
    }

    fn ensure_preview(&mut self, key: Key, an: Arc<Analysis>, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview.as_ref().is_some_and(|(k, _)| *k == key) || self.rendering {
            return;
        }
        self.rendering = true;
        let focus = key.area.and_then(|i| an.areas.get(i)).copied();
        let region = match focus {
            Some(r) => zoom_region(r, an.doc.mm_per_unit),
            None => an.doc.view,
        };
        let _ = window;
        cx.spawn(async move |this, cx| {
            let k = key.clone();
            let image = cx
                .background_spawn(async move {
                    let pm = crate::render::render(
                        &an,
                        &crate::render::View {
                            region,
                            width: k.w,
                            height: k.h,
                            focus,
                        },
                        k.dark,
                    );
                    to_render_image(pm)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.rendering = false;
                if let Some((_, old)) = this.preview.replace((key, Arc::new(image))) {
                    cx.drop_image(old, None);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn fix_one(path: &Path, an: &Analysis) -> Result<PathBuf, String> {
    let out = crate::fix::write_fixed(an, path).map_err(|e| format!("{e:#}"))?;
    crate::store::mark_clean(&out);
    Ok(out)
}

/// Area to show when zoomed in on a problem: the cluster with some context,
/// never narrower than ~40 mm so tiny ones are still readable.
fn zoom_region(r: Rect, mm_per_unit: f64) -> Rect {
    let min = 40.0 / mm_per_unit;
    let c = r.center();
    let w = (r.width() * 1.4).max(min);
    let h = (r.height() * 1.4).max(min);
    Rect::from_center_size(c, (w, h))
}

fn to_render_image(pm: tiny_skia::Pixmap) -> RenderImage {
    let (w, h) = (pm.width(), pm.height());
    let mut data = pm.take();
    // tiny-skia gives RGBA, gpui wants BGRA.
    for px in data.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let buf = image::RgbaImage::from_raw(w, h, data).expect("pixmap size");
    RenderImage::new(vec![image::Frame::new(buf)])
}

impl Render for Review {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(window);
        let s = self.settings;
        let n_bad = self
            .files
            .iter()
            .filter(|f| matches!(&f.state, State::Ready(a) if a.has_problems(&s)) && f.fixed.is_none())
            .count();
        let checking = self
            .files
            .iter()
            .filter(|f| matches!(f.state, State::Checking))
            .count();

        let summary = if self.files.is_empty() {
            "Drop SVGs or folders anywhere in this window".to_string()
        } else if checking > 0 {
            format!("Checking {checking} of {}…", self.files.len())
        } else if n_bad > 0 {
            format!("{n_bad} of {} files have double lines", self.files.len())
        } else {
            format!("{} files, no double lines left", self.files.len())
        };

        let header = div()
            .h(px(HEADER))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child("Laser Check"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(if n_bad > 0 { t.red } else { t.muted })
                            .child(summary),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("add", "Add files…", t)
                            .on_click(cx.listener(|_, _, _, cx| prompt_and_review(cx))),
                    )
                    .when(n_bad > 1, |d| {
                        d.child(
                            primary("fix-all", format!("Fix all {n_bad}"), t)
                                .on_click(cx.listener(|this, _, _, cx| this.fix_all(cx))),
                        )
                    }),
            );

        let body = if self.files.is_empty() {
            div().flex_1().p_6().flex().child(
                div()
                    .id("dropzone")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_dashed()
                    .border_color(t.border)
                    .rounded_xl()
                    .text_color(t.muted)
                    .child(
                        div()
                            .text_xl()
                            .text_color(t.text)
                            .child("Drop SVG files or folders here"),
                    )
                    .child("Each file is checked for lines the laser would cut more than once.")
                    .child(
                        button("choose", "Choose files…", t)
                            .on_click(cx.listener(|_, _, _, cx| prompt_and_review(cx))),
                    ),
            )
        } else {
            let viewport = window.viewport_size();
            let scale = window.scale_factor();
            let pw = (f32::from(viewport.width) - SIDEBAR - 32.0).max(100.0);
            let ph = (f32::from(viewport.height) - HEADER - TOOLBAR - 32.0).max(100.0);

            let sidebar = div()
                .id("files")
                .w(px(SIDEBAR))
                .flex_shrink_0()
                .h_full()
                .overflow_y_scroll()
                .bg(t.panel)
                .border_r_1()
                .border_color(t.border)
                .children(self.files.iter().enumerate().map(|(ix, f)| {
                    let selected = self.selected == Some(ix);
                    let (status, color) = status_line(f, &s, t);
                    div()
                        .id(("file", ix))
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(t.border)
                        .cursor_pointer()
                        .when(selected, |d| d.bg(t.button))
                        .hover(move |d| d.bg(t.button_hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(name(&f.path)),
                        )
                        .child(div().text_xs().text_color(color).truncate().child(status))
                }));

            let main = self.main_pane(pw, ph, scale, t, window, cx);
            div().flex_1().flex().min_h_0().child(sidebar).child(main)
        };

        div()
            .id("root")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add(paths.paths().to_vec(), cx)))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(t.red_soft))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .child(header)
            .child(body)
    }
}

fn status_line(f: &Item, s: &Settings, t: Theme) -> (String, Hsla) {
    match (&f.state, &f.fixed) {
        (_, Some(Ok(out))) => (format!("Fixed → {}", name(out)), t.green),
        (_, Some(Err(e))) => (format!("Fix failed: {e}"), t.red),
        (State::Checking, _) => ("Checking…".into(), t.muted),
        (State::Failed(e), _) => (format!("Couldn't read: {e}"), t.red),
        (State::Ready(a), _) if a.has_problems(s) => (
            format!("{} cut twice, {}", metres(a.removed_mm), areas(a.areas.len())),
            t.red,
        ),
        (State::Ready(_), _) => ("No double lines".into(), t.green),
    }
}

impl Review {
    fn main_pane(
        &mut self,
        pw: f32,
        ph: f32,
        scale: f32,
        t: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let s = self.settings;
        let pane = div().flex_1().min_w_0().flex().flex_col();
        let Some(ix) = self.selected else {
            return pane
                .items_center()
                .justify_center()
                .text_color(t.muted)
                .child("Checking…");
        };
        let item = &self.files[ix];
        let path = item.path.clone();
        let an = match &item.state {
            State::Ready(a) => a.clone(),
            State::Checking => {
                return pane
                    .items_center()
                    .justify_center()
                    .text_color(t.muted)
                    .child("Checking…");
            }
            State::Failed(e) => {
                return pane
                    .items_center()
                    .justify_center()
                    .text_color(t.red)
                    .child(format!("Couldn't read this file: {e}"));
            }
        };
        let bad = an.has_problems(&s);
        let fixed = item.fixed.clone();
        let n_areas = an.areas.len();

        let toolbar = div()
            .h(px(TOOLBAR))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name(&path)),
            )
            .when(bad && n_areas > 0, |d| {
                let label = match self.area {
                    Some(a) => format!("Area {} of {n_areas}", a + 1),
                    None => areas(n_areas),
                };
                d.child(
                    button("whole", "Whole sheet", t).on_click(cx.listener(|this, _, _, cx| {
                        this.area = None;
                        cx.notify();
                    })),
                )
                .child(button("prev", "◀", t).on_click(cx.listener(|this, _, _, cx| this.step_area(-1, cx))))
                .child(div().text_sm().text_color(t.muted).child(label))
                .child(button("next", "▶", t).on_click(cx.listener(|this, _, _, cx| this.step_area(1, cx))))
            })
            .child(button("reveal", "Show in Finder", t).on_click({
                let target = match &fixed {
                    Some(Ok(out)) => out.clone(),
                    _ => path.clone(),
                };
                move |_, _, cx| cx.reveal_path(&target)
            }))
            .when(bad && fixed.is_none(), |d| {
                d.child(
                    primary("fix", "Fix it", t).on_click(cx.listener(move |this, _, _, cx| this.fix(ix, cx))),
                )
            });

        let w = (pw * scale) as u32;
        let h = (ph * scale) as u32;
        let key = Key {
            path,
            area: self.area,
            w,
            h,
            dark: is_dark(window),
        };
        self.ensure_preview(key, an.clone(), window, cx);

        let note = if let Some(Ok(out)) = &fixed {
            format!(
                "Fixed copy saved as {}. Red shows where the original had double lines.",
                name(out)
            )
        } else if bad {
            format!(
                "Red lines are drawn more than once. Fixing keeps the first copy and saves {}.",
                name(&crate::fix::fixed_path(&self.files[ix].path))
            )
        } else {
            "Every line here is cut once.".into()
        };

        let preview = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_4()
            .gap_2()
            .child(match &self.preview {
                Some((_, image)) => div().child(
                    img(image.clone())
                        .w(px(pw))
                        .h(px(ph - 24.0))
                        .object_fit(ObjectFit::Contain),
                ),
                None => div().text_color(t.muted).child("Drawing…"),
            })
            .child(div().text_xs().text_color(t.muted).child(note));

        pane.child(toolbar).child(preview)
    }
}

// ---------------------------------------------------------------- alert

pub struct Alert {
    items: Vec<(PathBuf, Arc<Analysis>)>,
    fixed: Vec<Result<PathBuf, String>>,
}

fn open_alert(paths: Vec<PathBuf>, cx: &mut App) {
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
    let sound = Config::load().alert_sound;
    if !sound.is_empty() {
        let _ = std::process::Command::new("/usr/bin/afplay")
            .arg(format!("/System/Library/Sounds/{sound}.aiff"))
            .spawn();
    }
    let h = 250.0 + 22.0 * items.len().min(5) as f32;
    let bounds = Bounds::centered(None, size(px(560.), px(h)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Laser Check".into()),
                ..Default::default()
            }),
            // Floats above everything else until dealt with.
            kind: WindowKind::PopUp,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            ..Default::default()
        },
        move |_, cx| cx.new(|_| Alert { items, fixed: vec![] }),
    );
}

impl Alert {
    fn fix(&mut self) {
        if self.fixed.is_empty() {
            self.fixed = self.items.iter().map(|(p, an)| fix_one(p, an)).collect();
        }
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<Item> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, (p, an))| Item {
                path: p.clone(),
                state: State::Ready(an.clone()),
                fixed: self.fixed.get(i).cloned(),
            })
            .collect();
        // Opening a window inside this click handler deadlocks (the new window
        // becoming key calls back into this one mid-update), so do it after.
        // Close the alert only once the review is up, or the app sees no
        // windows left and quits.
        let alert = window.window_handle();
        cx.defer(move |cx| {
            open_review(vec![], items, cx);
            let _ = alert.update(cx, |_, window, _| window.remove_window());
        });
    }
}

impl Render for Alert {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(window);
        let n = self.items.len();
        let done = !self.fixed.is_empty();
        let total: f64 = self.items.iter().map(|(_, a)| a.removed_mm).sum();
        let title = if done {
            "Fixed".to_string()
        } else if n == 1 {
            "Double lines in this SVG".to_string()
        } else {
            format!("Double lines in {n} SVGs")
        };
        let subtitle = if done {
            let ok = self.fixed.iter().filter(|f| f.is_ok()).count();
            format!(
                "Saved {ok} fixed {} next to the original{}. The originals are untouched.",
                if ok == 1 { "copy" } else { "copies" },
                if ok == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "The laser would go over {} of line more than once.",
                metres(total)
            )
        };

        let list = div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.items.iter().take(5).enumerate().map(|(i, (p, an))| {
                let right = match self.fixed.get(i) {
                    Some(Ok(out)) => format!("→ {}", name(out)),
                    Some(Err(e)) => format!("fix failed: {e}"),
                    None => metres(an.removed_mm),
                };
                div()
                    .flex()
                    .justify_between()
                    .gap_4()
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name(p)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if done { t.green } else { t.red })
                            .child(right),
                    )
            }));
        let more = (n > 5).then(|| {
            div()
                .text_xs()
                .text_color(t.muted)
                .child(format!("and {} more", n - 5))
        });

        let buttons = if done {
            let first = self.fixed.iter().find_map(|f| f.as_ref().ok().cloned());
            div()
                .flex()
                .gap_2()
                .justify_end()
                .when_some(first, |d, out| {
                    d.child(
                        button("finder", "Show in Finder", t).on_click(move |_, _, cx| cx.reveal_path(&out)),
                    )
                })
                .child(
                    button("show", "Show me", t)
                        .on_click(cx.listener(|this, _, window, cx| this.show(window, cx))),
                )
                .child(
                    primary("done", "Done", t)
                        .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                )
        } else {
            div()
                .flex()
                .gap_2()
                .justify_end()
                .child(
                    button("ignore", "Ignore", t)
                        .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                )
                .child(
                    button("show", "Show me", t)
                        .on_click(cx.listener(|this, _, window, cx| this.show(window, cx))),
                )
                .child(button("fix-show", "Fix and show me", t).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.fix();
                        this.show(window, cx);
                    },
                )))
                .child(
                    primary("fix", "Fix it", t).on_click(cx.listener(|this, _, _, cx| {
                        this.fix();
                        cx.notify();
                    })),
                )
        };

        let accent = if done { t.green } else { t.red };
        div()
            .size_full()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .child(div().w(px(10.)).h_full().bg(accent))
            .child(
                div()
                    .flex_1()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(34.))
                                    .rounded_full()
                                    .bg(accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(gpui::white())
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(if done { "✓" } else { "!" }),
                            )
                            .child(div().text_2xl().font_weight(FontWeight::BOLD).child(title)),
                    )
                    .child(div().text_sm().text_color(t.muted).child(subtitle))
                    .child(div().flex_1().child(list).children(more))
                    .child(buttons),
            )
    }
}
