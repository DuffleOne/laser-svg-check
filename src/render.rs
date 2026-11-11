//! Rasterises the cut lines with doubled stretches in red.

use kurbo::{ParamCurve, ParamCurveExtrema, PathSeg, Rect};
use tiny_skia::{Color, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::analyze::Analysis;

pub struct View {
    /// Region of the drawing to show, in root user units.
    pub region: Rect,
    pub width: u32,
    pub height: u32,
    /// Area to outline, e.g. the one being looked at.
    pub focus: Option<Rect>,
}

/// Scale and offset that fit `region` into a `w`x`h` canvas, keeping its
/// aspect ratio and centring it. The UI uses the same numbers to place markers.
pub fn fit(region: Rect, w: f64, h: f64) -> (f64, f64, f64) {
    let s = (w / region.width()).min(h / region.height());
    let dx = (w - region.width() * s) / 2.0 - region.x0 * s;
    let dy = (h - region.height() * s) / 2.0 - region.y0 * s;
    (s, dx, dy)
}

struct Palette {
    bg: [u8; 3],
    line: [u8; 3],
    edge: [u8; 3],
    double: [u8; 3],
    focus: [u8; 3],
    /// Wide, faint strokes drawn under doubled lines as a glow: (alpha, width in px).
    glow: &'static [(u8, f32)],
}

const fn rgb(r: u8, g: u8, b: u8) -> [u8; 3] {
    [r, g, b]
}

fn color([r, g, b]: [u8; 3], alpha: u8) -> Color {
    Color::from_rgba8(r, g, b, alpha)
}

fn stroke(pm: &mut Pixmap, path: &tiny_skia::Path, c: Color, width: f32, ts: Transform) {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    let st = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
    pm.stroke_path(path, &paint, &st, ts, None);
}

const DARK: Palette = Palette {
    bg: rgb(0x10, 0x13, 0x1D),
    line: rgb(0x70, 0x79, 0x96),
    edge: rgb(0x26, 0x2B, 0x3D),
    double: rgb(0xFF, 0x48, 0x58),
    focus: rgb(0xFF, 0x6A, 0x1A),
    glow: &[(28, 11.0), (60, 6.0)],
};

const LIGHT: Palette = Palette {
    bg: rgb(255, 255, 255),
    line: rgb(110, 110, 115),
    edge: rgb(215, 215, 220),
    double: rgb(235, 40, 50),
    focus: rgb(255, 170, 0),
    glow: &[],
};

pub fn render(an: &Analysis, v: &View, dark: bool) -> Pixmap {
    let pal = if dark { &DARK } else { &LIGHT };
    let mut pm = Pixmap::new(v.width.max(1), v.height.max(1)).unwrap();
    let (bg, line) = (color(pal.bg, 255), color(pal.line, 255));
    pm.fill(bg);
    let (s, dx, dy) = fit(v.region, v.width as f64, v.height as f64);
    let ts = Transform::from_row(s as f32, 0.0, 0.0, s as f32, dx as f32, dy as f32);
    let scale = ts.sx as f64;
    let visible = v
        .region
        .inflate(v.region.width() * 0.05, v.region.height() * 0.05);

    // Faint sheet edge so the artboard is visible against the background.
    let v0 = an.doc.view;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(v0.x0 as f32, v0.y0 as f32, v0.x1 as f32, v0.y1 as f32) {
        let mut paint = Paint::default();
        paint.set_color(color(pal.edge, 255));
        let stroke = Stroke {
            width: 1.0 / ts.sx,
            ..Default::default()
        };
        pm.stroke_path(&PathBuilder::from_rect(rect), &paint, &stroke, ts, None);
    }

    // All lines, one path per element.
    let mut pb = PathBuilder::new();
    let mut last_el = usize::MAX;
    let flush = |pb: &mut PathBuilder, pm: &mut Pixmap, color: Color, width: f32| {
        let b = std::mem::replace(pb, PathBuilder::new());
        if let Some(path) = b.finish() {
            stroke(pm, &path, color, width, ts);
        }
    };
    for (i, sg) in an.doc.segs.iter().enumerate() {
        if !overlaps(sg.seg.bounding_box(), visible) {
            continue;
        }
        if sg.el != last_el || i % 2000 == 0 {
            flush(&mut pb, &mut pm, line, 1.0 / scale as f32);
            last_el = sg.el;
        }
        add_seg(&mut pb, sg.seg, true);
    }
    flush(&mut pb, &mut pm, line, 1.0 / scale as f32);

    // Doubled stretches, thick and red so short ones still show at full-sheet
    // zoom, over a soft glow of wider, fainter strokes.
    let mut doubled = PathBuilder::new();
    for p in &an.patches {
        let seg = an.doc.segs[p.seg].seg;
        if !overlaps(seg.bounding_box(), visible) {
            continue;
        }
        let f = &an.flat[p.seg];
        add_seg(&mut doubled, seg.subsegment(f.t_at(p.a0)..f.t_at(p.a1)), true);
    }
    if let Some(path) = doubled.finish() {
        for &(alpha, width) in pal.glow {
            stroke(&mut pm, &path, color(pal.double, alpha), width / scale as f32, ts);
        }
        stroke(&mut pm, &path, color(pal.double, 255), 2.5 / scale as f32, ts);
    }

    if let Some(r) = v.focus {
        let pad = 6.0 / scale;
        let r = r.inflate(pad, pad);
        if let Some(rect) = tiny_skia::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32) {
            pb.push_rect(rect);
        }
        flush(&mut pb, &mut pm, color(pal.focus, 255), 1.5 / scale as f32);
    }
    pm
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && a.x1 >= b.x0 && a.y0 <= b.y1 && a.y1 >= b.y0
}

fn add_seg(pb: &mut PathBuilder, seg: PathSeg, move_first: bool) {
    let f = |v: f64| v as f32;
    let p0 = seg.start();
    if move_first {
        pb.move_to(f(p0.x), f(p0.y));
    }
    match seg {
        PathSeg::Line(l) => pb.line_to(f(l.p1.x), f(l.p1.y)),
        PathSeg::Quad(q) => pb.quad_to(f(q.p1.x), f(q.p1.y), f(q.p2.x), f(q.p2.y)),
        PathSeg::Cubic(c) => pb.cubic_to(f(c.p1.x), f(c.p1.y), f(c.p2.x), f(c.p2.y), f(c.p3.x), f(c.p3.y)),
    }
}
