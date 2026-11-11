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

/// Fits `region` into a `w`x`h` canvas, preserving aspect ratio.
fn fit(region: Rect, w: u32, h: u32) -> Transform {
    let s = (w as f64 / region.width()).min(h as f64 / region.height());
    let dx = (w as f64 - region.width() * s) / 2.0 - region.x0 * s;
    let dy = (h as f64 - region.height() * s) / 2.0 - region.y0 * s;
    Transform::from_row(s as f32, 0.0, 0.0, s as f32, dx as f32, dy as f32)
}

pub fn render(an: &Analysis, v: &View, dark: bool) -> Pixmap {
    let mut pm = Pixmap::new(v.width.max(1), v.height.max(1)).unwrap();
    let (bg, line) = if dark {
        (
            Color::from_rgba8(30, 30, 32, 255),
            Color::from_rgba8(150, 150, 155, 255),
        )
    } else {
        (
            Color::from_rgba8(255, 255, 255, 255),
            Color::from_rgba8(110, 110, 115, 255),
        )
    };
    pm.fill(bg);
    let ts = fit(v.region, v.width, v.height);
    let scale = ts.sx as f64;
    let visible = v
        .region
        .inflate(v.region.width() * 0.05, v.region.height() * 0.05);

    // Faint sheet edge so the artboard is visible against the background.
    let v0 = an.doc.view;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(v0.x0 as f32, v0.y0 as f32, v0.x1 as f32, v0.y1 as f32) {
        let mut paint = Paint::default();
        paint.set_color(if dark {
            Color::from_rgba8(70, 70, 76, 255)
        } else {
            Color::from_rgba8(215, 215, 220, 255)
        });
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
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            let stroke = Stroke {
                width,
                line_cap: LineCap::Round,
                ..Default::default()
            };
            pm.stroke_path(&path, &paint, &stroke, ts, None);
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

    // Doubled stretches, thick and red so short ones still show at full-sheet zoom.
    let red = Color::from_rgba8(235, 40, 50, 255);
    for p in &an.patches {
        let seg = an.doc.segs[p.seg].seg;
        if !overlaps(seg.bounding_box(), visible) {
            continue;
        }
        let f = &an.flat[p.seg];
        add_seg(&mut pb, seg.subsegment(f.t_at(p.a0)..f.t_at(p.a1)), true);
    }
    flush(&mut pb, &mut pm, red, 3.5 / scale as f32);

    if let Some(r) = v.focus {
        let pad = 6.0 / scale;
        let r = r.inflate(pad, pad);
        if let Some(rect) = tiny_skia::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32) {
            pb.push_rect(rect);
        }
        flush(
            &mut pb,
            &mut pm,
            Color::from_rgba8(255, 170, 0, 255),
            2.0 / scale as f32,
        );
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
