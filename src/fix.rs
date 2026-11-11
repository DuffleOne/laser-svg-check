//! Writes a copy of the SVG with the later copy of every doubled stretch cut
//! out. Only touched elements are rewritten; everything else (layers, ids,
//! colours, text, images) is left byte for byte.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use kurbo::{Affine, ParamCurve, PathSeg, Point};

use crate::analyze::Analysis;

/// Path the fixed copy goes to: `name-fixed.svg` next to the original.
pub fn fixed_path(src: &Path) -> PathBuf {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    src.with_file_name(format!("{stem}-fixed.svg"))
}

pub fn write_fixed(an: &Analysis, src: &Path) -> Result<PathBuf> {
    let out = fixed_path(src);
    std::fs::write(&out, fixed_svg(an))?;
    Ok(out)
}

pub fn fixed_svg(an: &Analysis) -> String {
    let doc = &an.doc;
    let src = &doc.src;
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for el in &doc.elements {
        let touched = el
            .subpaths
            .iter()
            .any(|sp| sp.segs.clone().any(|i| !an.removals[i].is_empty()));
        if !touched {
            continue;
        }
        let inv = el.ctm.inverse();
        let mut d = String::new();
        for sp in &el.subpaths {
            for chain in kept_chains(an, sp.segs.clone(), sp.closed) {
                write_chain(&mut d, &chain, inv);
            }
        }
        out.push_str(&src[at..el.range.start]);
        if !d.is_empty() {
            out.push_str("<path");
            for r in &el.keep_attrs {
                out.push(' ');
                out.push_str(&src[r.clone()]);
            }
            let _ = write!(out, " d=\"{}\"/>", d.trim_end());
        } else {
            // Element vanished entirely: also eat the indentation before it.
            let trimmed = out.trim_end_matches([' ', '\t']).len();
            if out[..trimmed].ends_with('\n') {
                out.truncate(trimmed - 1);
            }
        }
        at = el.range.end;
    }
    out.push_str(&src[at..]);
    out
}

/// A run of connected segments to draw, plus whether it closes on itself.
struct Chain {
    segs: Vec<PathSeg>,
    closed: bool,
}

fn kept_chains(an: &Analysis, segs: std::ops::Range<usize>, closed: bool) -> Vec<Chain> {
    // Surviving fragments, tagged with their segment and whether they reach
    // its start and end, so neighbours can be joined back up.
    struct Frag {
        i: usize,
        seg: PathSeg,
        from_start: bool,
        to_end: bool,
    }
    let zero = |i: usize| an.flat[i].len() < 1e-9;
    let mut frags: Vec<Frag> = vec![];
    let mut intact = true;
    for i in segs.clone() {
        if !an.removals[i].is_empty() {
            intact = false;
        }
        // Zero-length segments (e.g. Z right after landing on the start
        // point) carry nothing; joining treats them as see-through.
        if zero(i) {
            continue;
        }
        let f = &an.flat[i];
        let seg = an.doc.segs[i].seg;
        let len = f.len();
        let mut cur = 0.0;
        let mut keep = vec![];
        for &(a0, a1) in &an.removals[i] {
            if a0 > cur {
                keep.push((cur, a0));
            }
            cur = cur.max(a1);
        }
        if cur < len {
            keep.push((cur, len));
        }
        for (a0, a1) in keep {
            let (t0, t1) = (f.t_at(a0), f.t_at(a1));
            let piece = if t0 == 0.0 && t1 == 1.0 {
                seg
            } else {
                seg.subsegment(t0..t1)
            };
            frags.push(Frag {
                i,
                seg: piece,
                from_start: a0 == 0.0,
                to_end: a1 >= len,
            });
        }
    }
    if intact {
        let all = segs.clone().map(|i| an.doc.segs[i].seg).collect();
        return vec![Chain { segs: all, closed }];
    }
    // Next to each other along the path, with only zero-length segments between.
    let adjacent = |a: usize, b: usize| a < b && (a + 1..b).all(zero);

    struct Run {
        segs: Vec<PathSeg>,
        first: usize,
        from_start: bool,
        last: usize,
        to_end: bool,
    }
    let mut runs: Vec<Run> = vec![];
    for f in frags {
        match runs.last_mut() {
            Some(r) if r.to_end && f.from_start && adjacent(r.last, f.i) => {
                r.segs.push(f.seg);
                r.last = f.i;
                r.to_end = f.to_end;
            }
            _ => runs.push(Run {
                segs: vec![f.seg],
                first: f.i,
                from_start: f.from_start,
                last: f.i,
                to_end: f.to_end,
            }),
        }
    }
    // On a closed loop the last run may carry on across the start into the first.
    if closed && runs.len() > 1 {
        let n = runs.len();
        let first_at_start = runs[0].from_start && (segs.start..runs[0].first).all(zero);
        let last_at_end = runs[n - 1].to_end && (runs[n - 1].last + 1..segs.end).all(zero);
        if first_at_start && last_at_end {
            let mut last = runs.pop().unwrap();
            last.segs.append(&mut runs[0].segs);
            runs[0].segs = last.segs;
        }
    }
    runs.into_iter()
        .map(|r| Chain {
            segs: r.segs,
            closed: false,
        })
        .collect()
}

fn write_chain(d: &mut String, chain: &Chain, inv: Affine) {
    let Some(first) = chain.segs.first() else { return };
    let p = |d: &mut String, pt: Point| {
        let pt = inv * pt;
        let _ = write!(d, "{},{} ", num(pt.x), num(pt.y));
    };
    d.push('M');
    p(d, first.start());
    let n = chain.segs.len();
    for (i, seg) in chain.segs.iter().enumerate() {
        // Closing line is implied by Z.
        if chain.closed && i == n - 1 && matches!(seg, PathSeg::Line(_)) {
            break;
        }
        match *seg {
            PathSeg::Line(l) => {
                d.push('L');
                p(d, l.p1);
            }
            PathSeg::Quad(q) => {
                d.push('Q');
                p(d, q.p1);
                p(d, q.p2);
            }
            PathSeg::Cubic(c) => {
                d.push('C');
                p(d, c.p1);
                p(d, c.p2);
                p(d, c.p3);
            }
        }
    }
    if chain.closed {
        d.push_str("Z ");
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
