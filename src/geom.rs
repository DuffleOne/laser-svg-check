//! Reads an SVG into flat geometry: every drawable element becomes a list of
//! subpaths made of line/quad/cubic segments in root user units, with enough
//! bookkeeping (byte range, transform) to splice a fixed version back in.

use std::ops::Range;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use kurbo::{Affine, BezPath, Circle, Ellipse, PathEl, PathSeg, Point, Rect, RoundedRect, Shape};

pub struct Doc {
    pub src: String,
    pub mm_per_unit: f64,
    /// Visible area in root user units (the viewBox).
    pub view: Rect,
    pub elements: Vec<Element>,
    pub segs: Vec<Seg>,
    /// Things we saw but don't look at, e.g. "2 text".
    pub skipped: Vec<String>,
}

pub struct Element {
    /// Byte range of the whole element in `src`.
    pub range: Range<usize>,
    /// Byte ranges of attributes to carry over when rewriting.
    pub keep_attrs: Vec<Range<usize>>,
    /// Element user space -> root user space.
    pub ctm: Affine,
    pub subpaths: Vec<Subpath>,
}

pub struct Subpath {
    pub segs: Range<usize>,
    pub closed: bool,
}

#[derive(Clone, Copy)]
pub struct Seg {
    pub el: usize,
    pub seg: PathSeg,
}

const GEOMETRY_ATTRS: &[&str] = &[
    "d",
    "points",
    "x",
    "y",
    "width",
    "height",
    "rx",
    "ry",
    "cx",
    "cy",
    "r",
    "x1",
    "y1",
    "x2",
    "y2",
    "pathLength",
];

const NON_RENDERED: &[&str] = &[
    "defs",
    "clipPath",
    "mask",
    "symbol",
    "pattern",
    "marker",
    "linearGradient",
    "radialGradient",
    "style",
    "title",
    "desc",
    "metadata",
    "script",
    "filter",
];

pub fn parse(src: String) -> Result<Doc> {
    let mut doc = Doc {
        src,
        mm_per_unit: 25.4 / 72.0,
        view: Rect::ZERO,
        elements: vec![],
        segs: vec![],
        skipped: vec![],
    };
    // roxmltree borrows the text, so collect into locals and move in after.
    let src = std::mem::take(&mut doc.src);
    {
        let xml = roxmltree::Document::parse_with_options(
            &src,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .context("not valid XML")?;
        let root = xml.root_element();
        if root.tag_name().name() != "svg" {
            bail!("root element is <{}>, not <svg>", root.tag_name().name());
        }
        let (view, mm_per_unit) = root_units(&root);
        doc.view = view;
        doc.mm_per_unit = mm_per_unit;

        let mut skipped = std::collections::BTreeMap::<String, usize>::new();
        for child in root.children().filter(|n| n.is_element()) {
            walk(child, Affine::IDENTITY, &mut doc, &mut skipped);
        }
        doc.skipped = skipped.into_iter().map(|(k, v)| format!("{v} <{k}>")).collect();
    }
    doc.src = src;
    Ok(doc)
}

fn root_units(root: &roxmltree::Node) -> (Rect, f64) {
    let len_mm = |name: &str| -> Option<f64> {
        let l = svgtypes::Length::from_str(root.attribute(name)?).ok()?;
        use svgtypes::LengthUnit::*;
        // Illustrator writes px meaning 1/72 in, so treat px and bare numbers as points.
        let per = match l.unit {
            Mm => 1.0,
            Cm => 10.0,
            In => 25.4,
            Pt | Px | None => 25.4 / 72.0,
            Pc => 25.4 / 6.0,
            _ => return Option::None,
        };
        Some(l.number * per)
    };
    let vb = root
        .attribute("viewBox")
        .and_then(|v| svgtypes::ViewBox::from_str(v).ok())
        .filter(|v| v.w > 0.0 && v.h > 0.0);
    match (vb, len_mm("width")) {
        (Some(vb), Some(w)) => (Rect::new(vb.x, vb.y, vb.x + vb.w, vb.y + vb.h), w / vb.w),
        (Some(vb), None) => (Rect::new(vb.x, vb.y, vb.x + vb.w, vb.y + vb.h), 25.4 / 72.0),
        (None, w) => {
            let num = |n: &str| {
                root.attribute(n)
                    .and_then(|v| svgtypes::Length::from_str(v).ok())
                    .map(|l| l.number)
                    .unwrap_or(100.0)
            };
            let (ww, hh) = (num("width"), num("height"));
            (
                Rect::new(0.0, 0.0, ww, hh),
                w.map(|w| w / ww).unwrap_or(25.4 / 72.0),
            )
        }
    }
}

fn hidden(node: &roxmltree::Node) -> bool {
    node.attribute("display") == Some("none")
        || node
            .attribute("style")
            .is_some_and(|s| s.replace(' ', "").contains("display:none"))
}

fn transform_of(node: &roxmltree::Node) -> Affine {
    node.attribute("transform")
        .and_then(|t| svgtypes::Transform::from_str(t).ok())
        .map(|t| Affine::new([t.a, t.b, t.c, t.d, t.e, t.f]))
        .unwrap_or(Affine::IDENTITY)
}

fn num(node: &roxmltree::Node, name: &str) -> f64 {
    node.attribute(name)
        .and_then(|v| svgtypes::Length::from_str(v).ok())
        .map(|l| l.number)
        .unwrap_or(0.0)
}

fn walk(
    node: roxmltree::Node,
    parent: Affine,
    doc: &mut Doc,
    skipped: &mut std::collections::BTreeMap<String, usize>,
) {
    let name = node.tag_name().name();
    if NON_RENDERED.contains(&name) || hidden(&node) {
        return;
    }
    let mut ctm = parent * transform_of(&node);
    let local: Option<BezPath> = match name {
        "g" | "a" | "switch" => {
            for child in node.children().filter(|n| n.is_element()) {
                walk(child, ctm, doc, skipped);
            }
            return;
        }
        "svg" => {
            ctm *= Affine::translate((num(&node, "x"), num(&node, "y")));
            for child in node.children().filter(|n| n.is_element()) {
                walk(child, ctm, doc, skipped);
            }
            return;
        }
        "path" => node.attribute("d").and_then(|d| BezPath::from_svg(d).ok()),
        "rect" => {
            let (x, y, w, h) = (
                num(&node, "x"),
                num(&node, "y"),
                num(&node, "width"),
                num(&node, "height"),
            );
            let (mut rx, mut ry) = (num(&node, "rx"), num(&node, "ry"));
            if rx == 0.0 {
                rx = ry;
            }
            if ry == 0.0 {
                ry = rx;
            }
            let r = Rect::new(x, y, x + w, y + h);
            if w <= 0.0 || h <= 0.0 {
                None
            } else if rx > 0.0 {
                // kurbo only does circular corners; good enough for laser parts.
                Some(RoundedRect::from_rect(r, rx.min(ry).min(w / 2.0).min(h / 2.0)).to_path(0.01))
            } else {
                Some(r.to_path(0.01))
            }
        }
        "circle" => {
            let r = num(&node, "r");
            (r > 0.0).then(|| Circle::new((num(&node, "cx"), num(&node, "cy")), r).to_path(0.001))
        }
        "ellipse" => {
            let (rx, ry) = (num(&node, "rx"), num(&node, "ry"));
            (rx > 0.0 && ry > 0.0)
                .then(|| Ellipse::new((num(&node, "cx"), num(&node, "cy")), (rx, ry), 0.0).to_path(0.001))
        }
        "line" => {
            let mut p = BezPath::new();
            p.move_to((num(&node, "x1"), num(&node, "y1")));
            p.line_to((num(&node, "x2"), num(&node, "y2")));
            Some(p)
        }
        "polyline" | "polygon" => node.attribute("points").map(|pts| {
            let mut p = BezPath::new();
            for (i, (x, y)) in svgtypes::PointsParser::from(pts).enumerate() {
                if i == 0 {
                    p.move_to((x, y));
                } else {
                    p.line_to((x, y));
                }
            }
            if name == "polygon" {
                p.close_path();
            }
            p
        }),
        other => {
            *skipped.entry(other.to_string()).or_default() += 1;
            return;
        }
    };
    let Some(local) = local else { return };

    let keep_attrs = node
        .attributes()
        .filter(|a| a.namespace().is_some() || !GEOMETRY_ATTRS.contains(&a.name()))
        .map(|a| a.range())
        .collect();
    let el = doc.elements.len();
    let mut subpaths = vec![];
    let mut start = doc.segs.len();
    let mut first = Point::ZERO;
    let mut last = Point::ZERO;
    let mut open = false;
    let push = |doc: &mut Doc, seg: PathSeg| doc.segs.push(Seg { el, seg: ctm * seg });
    for pe in local.elements() {
        match *pe {
            PathEl::MoveTo(p) => {
                if open && doc.segs.len() > start {
                    subpaths.push(Subpath {
                        segs: start..doc.segs.len(),
                        closed: false,
                    });
                }
                start = doc.segs.len();
                first = p;
                last = p;
                open = true;
            }
            PathEl::LineTo(p) => {
                push(doc, PathSeg::Line(kurbo::Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(a, b) => {
                push(doc, PathSeg::Quad(kurbo::QuadBez::new(last, a, b)));
                last = b;
            }
            PathEl::CurveTo(a, b, c) => {
                push(doc, PathSeg::Cubic(kurbo::CubicBez::new(last, a, b, c)));
                last = c;
            }
            PathEl::ClosePath => {
                // Always emit the closing edge, even if zero length, so a
                // closed subpath's segments form a full loop.
                push(doc, PathSeg::Line(kurbo::Line::new(last, first)));
                if doc.segs.len() > start {
                    subpaths.push(Subpath {
                        segs: start..doc.segs.len(),
                        closed: true,
                    });
                }
                start = doc.segs.len();
                last = first;
                // A draw command straight after Z starts a new subpath at `first`.
                open = true;
            }
        }
    }
    if open && doc.segs.len() > start {
        subpaths.push(Subpath {
            segs: start..doc.segs.len(),
            closed: false,
        });
    }
    doc.elements.push(Element {
        range: node.range(),
        keep_attrs,
        ctm,
        subpaths,
    });
}
