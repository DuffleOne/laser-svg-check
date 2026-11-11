//! Design tokens and the small building blocks every window uses: icons,
//! buttons, mono text and number formatting.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use gpui::{
    BoxShadow, Div, ElementId, FontWeight, Hsla, Image, ImageFormat, ImageSource, SharedString, Stateful,
    Styled, Svg, Window, WindowAppearance, div, img, point, prelude::*, px, rgb, svg,
};

/// Design tokens (see design/README.md). Dark is the designed theme; light
/// keeps the old values until it gets its own pass.
#[derive(Clone, Copy)]
pub struct Theme {
    /// Window background.
    pub bg: Hsla,
    /// Behind the sheet preview.
    pub canvas: Hsla,
    /// Sidebar, inspector, cards.
    pub panel: Hsla,
    /// Selected row, secondary buttons.
    pub raised: Hsla,
    /// Hover, active segment.
    pub hover: Hsla,
    /// 1px dividers.
    pub border: Hsla,
    /// Outlines on cards and the selected row.
    pub outline: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    /// Section labels, hints.
    pub dim: Hsla,
    /// Primary actions and the selected area.
    pub laser: Hsla,
    /// Text on a laser-coloured button.
    pub on_laser: Hsla,
    /// Doubled lines, problem status.
    pub double: Hsla,
    /// Problem text on dark.
    pub double_text: Hsla,
    /// Numbers inside area markers.
    pub marker_text: Hsla,
    /// Clean or fixed.
    pub ok: Hsla,
}

pub fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

pub fn is_dark(window: &Window) -> bool {
    matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

pub fn theme(window: &Window) -> Theme {
    if is_dark(window) {
        Theme {
            bg: hex(0x0B0D14),
            canvas: hex(0x10131D),
            panel: hex(0x141826),
            raised: hex(0x1C2132),
            hover: hex(0x242A3E),
            border: hex(0x232838),
            outline: hex(0x2E3448),
            text: hex(0xECEEF4),
            muted: hex(0x8A91A8),
            dim: hex(0x5A6280),
            laser: hex(0xFF6A1A),
            on_laser: hex(0x1A0A02),
            double: hex(0xFF4858),
            double_text: hex(0xFF8A94),
            marker_text: hex(0xFFB3BA),
            ok: hex(0x5BD6A0),
        }
    } else {
        Theme {
            bg: hex(0xffffff),
            canvas: hex(0xffffff),
            panel: hex(0xf4f4f6),
            raised: hex(0xe8e8ec),
            hover: hex(0xdadade),
            border: hex(0xdcdce0),
            outline: hex(0xdcdce0),
            text: hex(0x1d1d1f),
            muted: hex(0x6e6e73),
            dim: hex(0x8e8e93),
            laser: hex(0xd70015),
            on_laser: hex(0xffffff),
            double: hex(0xd70015),
            double_text: hex(0xd70015),
            marker_text: hex(0xd70015),
            ok: hex(0x248a3d),
        }
    }
}

pub const MONO: &str = "JetBrains Mono";

/// Text in the mono face, for numbers, lengths and paths.
pub fn mono(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .text_size(px(size))
        .text_color(color)
        .child(text.into())
}

pub fn text(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div().text_size(px(size)).text_color(color).child(text.into())
}

/// One of the Lucide icons in assets/icons, tinted.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

/// The app icon, cropped to its tile.
pub fn app_icon(size: f32) -> impl IntoElement {
    static IMAGE: OnceLock<Arc<Image>> = OnceLock::new();
    let image = IMAGE
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/icon-128.png").to_vec(),
            ))
        })
        .clone();
    img(ImageSource::Image(image)).size(px(size)).flex_shrink_0()
}

pub fn shadow(color: Hsla, y: f32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color,
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    }]
}

/// Sets a border of any width, for the 1.5px outlines gpui has no helper for.
pub fn border_width<E: Styled>(mut e: E, w: f32) -> E {
    let b = &mut e.style().border_widths;
    b.top = Some(px(w).into());
    b.bottom = Some(px(w).into());
    b.left = Some(px(w).into());
    b.right = Some(px(w).into());
    e
}

fn base_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .cursor_pointer()
}

/// The orange action button. `label` may be followed by a shortcut hint.
pub fn primary_button(
    id: impl Into<ElementId>,
    icon_name: &str,
    label: impl Into<SharedString>,
    t: Theme,
) -> Stateful<Div> {
    let hover = t.laser.opacity(0.88);
    base_button(id)
        .px(px(14.))
        .py(px(7.))
        .bg(t.laser)
        .shadow(shadow(t.laser.opacity(0.25), 2., 16.))
        .text_color(t.on_laser)
        .font_weight(FontWeight::SEMIBOLD)
        .hover(move |s| s.bg(hover))
        .child(icon(icon_name, 14., t.on_laser))
        .child(label.into())
}

/// Raised, outlined button for second choices.
pub fn secondary_button(id: impl Into<ElementId>, label: impl Into<SharedString>, t: Theme) -> Stateful<Div> {
    base_button(id)
        .px(px(14.))
        .py(px(7.))
        .bg(t.raised)
        .border_1()
        .border_color(t.border)
        .text_color(t.text)
        .font_weight(FontWeight::MEDIUM)
        .hover(move |s| s.bg(t.hover))
        .child(label.into())
}

/// Plain text that acts as a button, for the least likely choice.
pub fn text_button(id: impl Into<ElementId>, label: impl Into<SharedString>, t: Theme) -> Stateful<Div> {
    base_button(id)
        .px(px(4.))
        .py(px(7.))
        .text_color(t.muted)
        .hover(move |s| s.text_color(t.text))
        .child(label.into())
}

/// A square outlined button holding just an icon.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_name: &str,
    size: f32,
    pad: f32,
    t: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .p(px(pad))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .cursor_pointer()
        .hover(move |s| s.bg(t.hover))
        .child(icon(icon_name, size, t.muted))
}

/// A length for the UI: "2.1 mm", "17 mm", "45.31 m".
pub fn length(mm: f64) -> String {
    if mm >= 1000.0 {
        format!("{:.2} m", mm / 1000.0)
    } else if mm >= 9.95 {
        format!("{mm:.0} mm")
    } else {
        format!("{mm:.1} mm")
    }
}

/// A length for a sentence, where "42 cm" reads better than "422 mm".
pub fn length_words(mm: f64) -> String {
    if (100.0..1000.0).contains(&mm) {
        format!("{:.0} cm", mm / 10.0)
    } else {
        length(mm)
    }
}

pub fn areas(n: usize) -> String {
    if n == 1 {
        "1 area".into()
    } else {
        format!("{n} areas")
    }
}

pub fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The folder a file is in, with the home folder shortened to `~`.
pub fn folder(p: &Path) -> String {
    let dir = p.parent().unwrap_or(p);
    let home = crate::config::home();
    match dir.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}
