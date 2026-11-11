//! Reads an SVG into flat geometry: every drawable element becomes a list of
//! subpaths made of line/quad/cubic segments in root user units, with enough
//! bookkeeping (byte range, transform) to splice a fixed version back in.

use std::collections::HashMap;
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

        let css = parse_css(&xml);
        let paint = paint_of(&root, Paint::default(), &css);
        let mut skipped = std::collections::BTreeMap::<String, usize>::new();
        for child in root.children().filter(|n| n.is_element()) {
            walk(child, Affine::IDENTITY, paint, &css, &mut doc, &mut skipped);
        }
        doc.skipped = skipped
            .into_iter()
            .map(|(k, v)| {
                if v == 1 || k.starts_with('<') {
                    format!("{v} {k}")
                } else {
                    format!("{v} {k}s")
                }
            })
            .collect();
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

/// What an element would actually draw. Laser software (LightBurn included)
/// only picks up shapes that have a stroke or a fill; a path with neither is
/// invisible and never cut, so it mustn't count as a copy of anything.
#[derive(Clone, Copy)]
struct Paint {
    fill: bool,
    stroke: bool,
    visible: bool,
}

impl Default for Paint {
    // SVG defaults: black fill, no stroke.
    fn default() -> Self {
        Paint {
            fill: true,
            stroke: false,
            visible: true,
        }
    }
}

impl Paint {
    fn draws(self) -> bool {
        self.visible && (self.fill || self.stroke)
    }

    fn set(&mut self, key: &str, value: &str) {
        let v = value.trim();
        if v == "inherit" || v.is_empty() {
            return;
        }
        match key.trim() {
            "fill" => self.fill = v != "none" && v != "transparent",
            "stroke" => self.stroke = v != "none" && v != "transparent",
            "visibility" => self.visible = v == "visible",
            "opacity" if v.parse::<f64>().is_ok_and(|o| o <= 0.0) => self.visible = false,
            _ => {}
        }
    }
}

/// `<style>` rules, keyed by selector: `.cls-1` or a tag name. Enough for
/// what Illustrator and Inkscape write; anything fancier is ignored.
type Css = HashMap<String, Vec<(String, String)>>;

fn parse_css(xml: &roxmltree::Document) -> Css {
    let mut css = Css::new();
    for style in xml.descendants().filter(|n| n.tag_name().name() == "style") {
        let mut text: String = style.children().filter_map(|c| c.text()).collect();
        while let Some(a) = text.find("/*") {
            let b = text[a..].find("*/").map(|b| a + b + 2).unwrap_or(text.len());
            text.replace_range(a..b, "");
        }
        for rule in text.split('}') {
            let Some((sels, body)) = rule.split_once('{') else {
                continue;
            };
            let decls: Vec<(String, String)> = body
                .split(';')
                .filter_map(|d| d.split_once(':'))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect();
            for sel in sels.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                css.entry(sel.to_string())
                    .or_default()
                    .extend(decls.iter().cloned());
            }
        }
    }
    css
}

/// Paint for `node`, given its parent's. Precedence low to high: inherited,
/// presentation attributes, stylesheet rules, the `style` attribute.
fn paint_of(node: &roxmltree::Node, parent: Paint, css: &Css) -> Paint {
    let mut p = parent;
    for key in ["fill", "stroke", "visibility", "opacity"] {
        if let Some(v) = node.attribute(key) {
            p.set(key, v);
        }
    }
    let mut rules = vec![node.tag_name().name().to_string()];
    if let Some(classes) = node.attribute("class") {
        rules.extend(classes.split_whitespace().map(|c| format!(".{c}")));
    }
    for (k, v) in rules.iter().filter_map(|r| css.get(r)).flatten() {
        p.set(k, v);
    }
    if let Some(style) = node.attribute("style") {
        for (k, v) in style.split(';').filter_map(|d| d.split_once(':')) {
            p.set(k, v);
        }
    }
    p
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
    parent_paint: Paint,
    css: &Css,
    doc: &mut Doc,
    skipped: &mut std::collections::BTreeMap<String, usize>,
) {
    let name = node.tag_name().name();
    if NON_RENDERED.contains(&name) || hidden(&node) {
        return;
    }
    let paint = paint_of(&node, parent_paint, css);
    let mut ctm = parent * transform_of(&node);
    let local: Option<BezPath> = match name {
        "g" | "a" | "switch" => {
            for child in node.children().filter(|n| n.is_element()) {
                walk(child, ctm, paint, css, doc, skipped);
            }
            return;
        }
        "svg" => {
            ctm *= Affine::translate((num(&node, "x"), num(&node, "y")));
            for child in node.children().filter(|n| n.is_element()) {
                walk(child, ctm, paint, css, doc, skipped);
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
            *skipped.entry(format!("<{other}>")).or_default() += 1;
            return;
        }
    };
    let Some(local) = local else { return };
    if !paint.draws() {
        *skipped.entry("invisible shape".into()).or_default() += 1;
        return;
    }

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
