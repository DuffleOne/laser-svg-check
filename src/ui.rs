//! The windows: a review window you can drop files and folders onto, and the
//! alert the watcher pops up.

mod alert;
mod review;
mod theme;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Application, AssetSource, Global, KeyBinding, Menu, MenuItem, PathPromptOptions, RenderImage,
    SharedString, WindowHandle, actions,
};

use crate::analyze::Analysis;
use crate::config::{Config, Settings};

pub use theme::{areas, length};

actions!(
    laser_check,
    [Quit, CloseWindow, OpenFiles, NextArea, PrevArea, FixFile]
);

pub enum Start {
    Review(Vec<PathBuf>),
    Alert(Vec<PathBuf>),
}

struct ReviewWindow(Option<WindowHandle<review::Review>>);
impl Global for ReviewWindow {}

/// Icons compiled into the binary, served to gpui's `svg()` element.
struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    ("icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    (
        "icons/check-large.svg",
        include_bytes!("../assets/icons/check-large.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-up.svg",
        include_bytes!("../assets/icons/chevron-up.svg"),
    ),
    (
        "icons/corner-down-right.svg",
        include_bytes!("../assets/icons/corner-down-right.svg"),
    ),
    ("icons/file.svg", include_bytes!("../assets/icons/file.svg")),
    (
        "icons/file-check.svg",
        include_bytes!("../assets/icons/file-check.svg"),
    ),
    (
        "icons/file-plus.svg",
        include_bytes!("../assets/icons/file-plus.svg"),
    ),
    (
        "icons/folder-open.svg",
        include_bytes!("../assets/icons/folder-open.svg"),
    ),
    (
        "icons/folder-search.svg",
        include_bytes!("../assets/icons/folder-search.svg"),
    ),
    ("icons/minus.svg", include_bytes!("../assets/icons/minus.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    ("icons/scan.svg", include_bytes!("../assets/icons/scan.svg")),
    (
        "icons/scan-search.svg",
        include_bytes!("../assets/icons/scan-search.svg"),
    ),
    (
        "icons/shield-check.svg",
        include_bytes!("../assets/icons/shield-check.svg"),
    ),
    (
        "icons/wand-sparkles.svg",
        include_bytes!("../assets/icons/wand-sparkles.svg"),
    ),
];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| (*p).into())
            .collect())
    }
}

pub fn run(start: Start) {
    let app = Application::new().with_assets(Assets);
    // Files opened from Finder ("Open With", or dropped on the Dock icon).
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| {
        let _ = tx.unbounded_send(urls);
    });
    app.run(move |cx: &mut App| {
        let _ = cx
            .text_system()
            .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect());
        cx.set_global(ReviewWindow(None));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenFiles, cx| prompt_and_review(cx));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-o", OpenFiles, None),
            KeyBinding::new("down", NextArea, Some("Review")),
            KeyBinding::new("up", PrevArea, Some("Review")),
            KeyBinding::new("cmd-enter", FixFile, Some("Review")),
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
                    MenuItem::action("Fix This File", FixFile),
                    MenuItem::action("Close Window", CloseWindow),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Next Area", NextArea),
                    MenuItem::action("Previous Area", PrevArea),
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
                let _ = cx.update(|cx| review::open(paths, vec![], cx));
            }
        })
        .detach();

        match start {
            Start::Review(paths) => review::open(paths, vec![], cx),
            Start::Alert(paths) => alert::open(paths, cx),
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

fn settings() -> Settings {
    Config::load().settings()
}

fn prompt_and_review(cx: &mut App) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: true,
        multiple: true,
        prompt: Some("Check".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            let _ = cx.update(|cx| review::open(paths, vec![], cx));
        }
    })
    .detach();
}

pub enum State {
    Checking,
    Ready(Arc<Analysis>),
    Failed(String),
}

/// A file in the review window, and its fixed copy if there is one.
pub struct Item {
    pub path: PathBuf,
    pub state: State,
    pub fixed: Option<Result<PathBuf, String>>,
}

fn fix_one(path: &Path, an: &Analysis) -> Result<PathBuf, String> {
    let out = crate::fix::write_fixed(an, path).map_err(|e| format!("{e:#}"))?;
    crate::store::mark_clean(&out);
    Ok(out)
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
