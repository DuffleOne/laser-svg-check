//! Finds stretches where two segments run along each other (within a
//! tolerance) instead of just crossing, and plans which copies to drop.

use std::collections::HashMap;

use kurbo::{ParamCurve, PathSeg, Point, Rect};

use crate::config::Settings;
use crate::geom::Doc;

pub struct Analysis {
    pub doc: Doc,
    pub flat: Vec<Flat>,
    /// Every doubled stretch, on every copy. Used for highlighting.
    pub patches: Vec<Patch>,
    /// Per segment, arc-length ranges the fix drops (later copies only).
    pub removals: Vec<Vec<(f64, f64)>>,
    pub removed_mm: f64,
    /// Bounding boxes of clusters of doubled lines, for "next problem".
    pub areas: Vec<Area>,
}

impl Analysis {
    pub fn has_problems(&self, s: &Settings) -> bool {
        self.removed_mm >= s.min_overlap_mm
    }
}

/// A segment flattened to a polyline, with curve parameter and arc length at
/// each vertex so arc ranges can be mapped back to curve parameters.
pub struct Flat {
    pub t: Vec<f64>,
    pub pts: Vec<Point>,
    pub arc: Vec<f64>,
}

impl Flat {
    pub fn len(&self) -> f64 {
        *self.arc.last().unwrap()
    }

    pub fn t_at(&self, a: f64) -> f64 {
        if a <= 0.0 {
            return 0.0;
        }
        let i = self.arc.partition_point(|&x| x < a);
        if i >= self.arc.len() {
            return 1.0;
        }
        let (a0, a1) = (self.arc[i - 1], self.arc[i]);
        let f = if a1 > a0 { (a - a0) / (a1 - a0) } else { 0.0 };
        self.t[i - 1] + f * (self.t[i] - self.t[i - 1])
    }

    fn bbox(&self, a0: f64, a1: f64) -> Rect {
        let mut r = Rect::from_points(self.point_at(a0), self.point_at(a1));
        for (p, &a) in self.pts.iter().zip(&self.arc) {
            if a > a0 && a < a1 {
                r = r.union_pt(*p);
            }
        }
        r
    }

    fn point_at(&self, a: f64) -> Point {
        let i = self.arc.partition_point(|&x| x < a).clamp(1, self.arc.len() - 1);
        let (a0, a1) = (self.arc[i - 1], self.arc[i]);
        let f = if a1 > a0 {
            ((a - a0) / (a1 - a0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.pts[i - 1].lerp(self.pts[i], f)
    }
}

/// A cluster of doubled lines, numbered in reading order in the UI.
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub rect: Rect,
    /// How much line the fix removes inside this area. The areas add up to
    /// `removed_mm`.
    pub mm: f64,
}

#[derive(Clone, Copy)]
pub struct Patch {
    pub seg: usize,
    pub a0: f64,
    pub a1: f64,
}

pub fn run(doc: Doc, s: &Settings) -> Analysis {
    let tol = s.tolerance_mm / doc.mm_per_unit;
    let cell = (2.0 / doc.mm_per_unit).max(tol * 4.0);
    let flat: Vec<Flat> = doc
        .segs
        .iter()
        .map(|sg| flatten(sg.seg, tol / 4.0, cell))
        .collect();

    // Spatial hash of polyline pieces.
    struct Piece {
        seg: u32,
        i: u32,
        bb: Rect,
    }
    let mut pieces = Vec::new();
    for (si, f) in flat.iter().enumerate() {
        for i in 0..f.pts.len() - 1 {
            let bb = Rect::from_points(f.pts[i], f.pts[i + 1]).inflate(tol, tol);
            pieces.push(Piece {
                seg: si as u32,
                i: i as u32,
                bb,
            });
        }
    }
    let key = |p: Point| ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (pi, p) in pieces.iter().enumerate() {
        let (x0, y0) = key(Point::new(p.bb.x0, p.bb.y0));
        let (x1, y1) = key(Point::new(p.bb.x1, p.bb.y1));
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                grid.entry((gx, gy)).or_default().push(pi as u32);
            }
        }
    }

    // covers[seg] = (arc0, arc1, by_seg): stretch of `seg` lying along `by_seg`.
    let sin_max = 25f64.to_radians().sin();
    let mut covers: Vec<Vec<(f64, f64, usize)>> = vec![vec![]; flat.len()];
    for (&k, list) in &grid {
        for (n, &a) in list.iter().enumerate() {
            for &b in &list[n + 1..] {
                let (pa, pb) = (&pieces[a as usize], &pieces[b as usize]);
                if pa.seg == pb.seg {
                    continue;
                }
                let inter = pa.bb.intersect(pb.bb);
                if inter.width() < 0.0 || inter.height() < 0.0 {
                    continue;
                }
                // Each pair only once: in the cell holding the overlap's corner.
                if key(Point::new(inter.x0, inter.y0)) != k {
                    continue;
                }
                let (fa, fb) = (&flat[pa.seg as usize], &flat[pb.seg as usize]);
                let (ia, ib) = (pa.i as usize, pb.i as usize);
                let (a0, a1) = (fa.pts[ia], fa.pts[ia + 1]);
                let (b0, b1) = (fb.pts[ib], fb.pts[ib + 1]);
                if let Some((s0, s1)) = cover(a0, a1, b0, b1, tol, sin_max) {
                    let base = fa.arc[ia];
                    covers[pa.seg as usize].push((base + s0, base + s1, pb.seg as usize));
                }
                if let Some((s0, s1)) = cover(b0, b1, a0, a1, tol, sin_max) {
                    let base = fb.arc[ib];
                    covers[pb.seg as usize].push((base + s0, base + s1, pa.seg as usize));
                }
            }
        }
    }

    let mut patches = vec![];
    let mut removals = vec![vec![]; flat.len()];
    let mut removed = 0.0;
    let min_len = s.min_overlap_mm / doc.mm_per_unit;
    // Measure doubled stretches along whole subpaths, not per segment: a long
    // doubled run is often many short segments, while sharp spike tips and
    // tangent touches only stay within tolerance for a fraction of a mm.
    let merged: Vec<Vec<(f64, f64)>> = covers
        .iter()
        .map(|c| merge(c.iter().map(|c| (c.0, c.1)).collect(), tol))
        .collect();
    let mut significant: Vec<Vec<(f64, f64)>> = vec![vec![]; flat.len()];
    for sp in doc.elements.iter().flat_map(|e| &e.subpaths) {
        let mut off = 0.0;
        let mut offs = vec![];
        let mut runs = vec![];
        for i in sp.segs.clone() {
            offs.push(off);
            runs.extend(merged[i].iter().map(|&(a, b)| (off + a, off + b)));
            off += flat[i].len();
        }
        if runs.is_empty() {
            continue;
        }
        let total = off;
        let runs = merge(runs, tol);
        let n = runs.len();
        // On a closed loop the first and last runs may be one run across the start.
        let wraps = sp.closed && n > 1 && runs[0].0 <= tol && runs[n - 1].1 >= total - tol;
        let wrap_len = if wraps {
            (runs[0].1 - runs[0].0) + (runs[n - 1].1 - runs[n - 1].0)
        } else {
            0.0
        };
        for (k, &(a, b)) in runs.iter().enumerate() {
            let len = if wraps && (k == 0 || k == n - 1) {
                wrap_len
            } else {
                b - a
            };
            if len < min_len {
                continue;
            }
            for (j, i) in sp.segs.clone().enumerate() {
                let (o, l) = (offs[j], flat[i].len());
                let (lo, hi) = (a.max(o) - o, b.min(o + l) - o);
                if hi > lo {
                    significant[i].push((lo, hi));
                }
            }
        }
    }

    for (si, cov) in covers.iter().enumerate() {
        let significant = &significant[si];
        if significant.is_empty() {
            continue;
        }
        let len = flat[si].len();
        patches.extend(significant.iter().map(|&(a0, a1)| Patch { seg: si, a0, a1 }));

        // Keep the first copy in document order, drop the later ones.
        let earlier = merge(cov.iter().filter(|c| c.2 < si).map(|c| (c.0, c.1)).collect(), tol);
        let mut drop = vec![];
        for (e0, e1) in earlier {
            for &(g0, g1) in significant {
                let (lo, hi) = (e0.max(g0), e1.min(g1));
                if hi > lo {
                    drop.push((lo, hi));
                }
            }
        }
        // Don't leave slivers at the ends.
        let mut drop = merge(drop, tol);
        for d in &mut drop {
            if d.0 <= tol {
                d.0 = 0.0;
            }
            if d.1 >= len - tol {
                d.1 = len;
            }
        }
        removed += drop.iter().map(|d| d.1 - d.0).sum::<f64>();
        removals[si] = drop;
    }

    let (rects, group) = cluster(&patches, &flat, 1.0 / doc.mm_per_unit);
    // Each removed stretch lies inside one of the doubled patches on its
    // segment; credit its length to that patch's area.
    let mut mm = vec![0.0; rects.len()];
    // Patches were pushed segment by segment, so each segment's are contiguous.
    let mut by_seg: HashMap<usize, std::ops::Range<usize>> = HashMap::new();
    for (i, p) in patches.iter().enumerate() {
        by_seg
            .entry(p.seg)
            .and_modify(|r| r.end = i + 1)
            .or_insert(i..i + 1);
    }
    for (si, drops) in removals.iter().enumerate() {
        let Some(range) = by_seg.get(&si) else { continue };
        for &(lo, hi) in drops {
            let mid = (lo + hi) / 2.0;
            let owner = range
                .clone()
                .min_by(|&a, &b| dist(&patches[a], mid).total_cmp(&dist(&patches[b], mid)))
                .map(|i| group[i]);
            if let Some(g) = owner {
                mm[g] += (hi - lo) * doc.mm_per_unit;
            }
        }
    }
    let areas = rects
        .into_iter()
        .zip(mm)
        .map(|(rect, mm)| Area { rect, mm })
        .collect();
    Analysis {
        removed_mm: removed * doc.mm_per_unit,
        doc,
        flat,
        patches,
        removals,
        areas,
    }
}

fn flatten(seg: PathSeg, flat_tol: f64, max_piece: f64) -> Flat {
    let (n_flat, hull) = match seg {
        PathSeg::Line(l) => (1.0, (l.p1 - l.p0).hypot()),
        PathSeg::Quad(q) => {
            let dd = (q.p0.to_vec2() - 2.0 * q.p1.to_vec2() + q.p2.to_vec2()).hypot();
            (
                (2.0 * dd / (8.0 * flat_tol)).sqrt(),
                (q.p1 - q.p0).hypot() + (q.p2 - q.p1).hypot(),
            )
        }
        PathSeg::Cubic(c) => {
            let d1 = (c.p0.to_vec2() - 2.0 * c.p1.to_vec2() + c.p2.to_vec2()).hypot();
            let d2 = (c.p1.to_vec2() - 2.0 * c.p2.to_vec2() + c.p3.to_vec2()).hypot();
            (
                (6.0 * d1.max(d2) / (8.0 * flat_tol)).sqrt(),
                (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot(),
            )
        }
    };
    let n = (n_flat.ceil().max((hull / max_piece).ceil()).max(1.0) as usize).min(4096);
    let mut f = Flat {
        t: Vec::with_capacity(n + 1),
        pts: Vec::with_capacity(n + 1),
        arc: Vec::with_capacity(n + 1),
    };
    let mut acc = 0.0;
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let p = seg.eval(t);
        if let Some(&prev) = f.pts.last() {
            acc += (p - prev).hypot();
        }
        f.t.push(t);
        f.pts.push(p);
        f.arc.push(acc);
    }
    f
}

/// Part of piece `a` (as arc range from a0) that has piece `b` lying along it
/// within `tol`, if they run roughly parallel (either direction).
fn cover(a0: Point, a1: Point, b0: Point, b1: Point, tol: f64, sin_max: f64) -> Option<(f64, f64)> {
    let d = a1 - a0;
    let len = d.hypot();
    let e = b1 - b0;
    let elen = e.hypot();
    if len < 1e-9 || elen < 1e-9 {
        return None;
    }
    let u = d / len;
    if (u.cross(e / elen)).abs() > sin_max {
        return None;
    }
    let along = |p: Point| (p - a0).dot(u);
    let off = |p: Point| u.cross(p - a0);
    let (s0, s1) = (along(b0), along(b1));
    let (h0, h1) = (off(b0), off(b1));
    let (lo1, hi1) = band(h0, h1, -tol, tol)?;
    let (lo2, hi2) = band(s0, s1, 0.0, len)?;
    let (lo, hi) = (lo1.max(lo2), hi1.min(hi2));
    if hi <= lo {
        return None;
    }
    let (x, y) = (s0 + lo * (s1 - s0), s0 + hi * (s1 - s0));
    let (x, y) = (x.min(y).clamp(0.0, len), x.max(y).clamp(0.0, len));
    (y - x > 1e-9).then_some((x, y))
}

/// Range of u in [0,1] where f0 + u (f1 - f0) lies within [lo, hi].
fn band(f0: f64, f1: f64, lo: f64, hi: f64) -> Option<(f64, f64)> {
    let df = f1 - f0;
    if df.abs() < 1e-12 {
        return (f0 >= lo && f0 <= hi).then_some((0.0, 1.0));
    }
    let (mut a, mut b) = ((lo - f0) / df, (hi - f0) / df);
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    let (a, b) = (a.max(0.0), b.min(1.0));
    (b > a).then_some((a, b))
}

fn merge(mut v: Vec<(f64, f64)>, gap: f64) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 + gap => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// How far arc position `a` is from patch `p` (0 inside it).
fn dist(p: &Patch, a: f64) -> f64 {
    (p.a0 - a).max(a - p.a1).max(0.0)
}

/// Groups patches whose boxes touch into areas. Returns the areas, sorted top
/// to bottom then left to right, and the area index of every patch.
fn cluster(patches: &[Patch], flat: &[Flat], pad: f64) -> (Vec<Rect>, Vec<usize>) {
    let boxes: Vec<Rect> = patches
        .iter()
        .map(|p| flat[p.seg].bbox(p.a0, p.a1).inflate(pad, pad))
        .collect();
    let mut parent: Vec<usize> = (0..boxes.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let cell = pad * 10.0;
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, b) in boxes.iter().enumerate() {
        for gx in (b.x0 / cell).floor() as i32..=(b.x1 / cell).floor() as i32 {
            for gy in (b.y0 / cell).floor() as i32..=(b.y1 / cell).floor() as i32 {
                grid.entry((gx, gy)).or_default().push(i);
            }
        }
    }
    for list in grid.values() {
        for (n, &a) in list.iter().enumerate() {
            for &b in &list[n + 1..] {
                let i = boxes[a].intersect(boxes[b]);
                if i.width() >= 0.0 && i.height() >= 0.0 {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut index_of_root: HashMap<usize, usize> = HashMap::new();
    let mut rects: Vec<Rect> = vec![];
    let mut group = vec![0; boxes.len()];
    for (i, b) in boxes.iter().enumerate() {
        let r = find(&mut parent, i);
        let g = *index_of_root.entry(r).or_insert_with(|| {
            rects.push(*b);
            rects.len() - 1
        });
        rects[g] = rects[g].union(*b);
        group[i] = g;
    }
    let rects: Vec<Rect> = rects.into_iter().map(|b| b.inflate(-pad, -pad)).collect();
    // Reading order: top to bottom by centre, then left to right.
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| {
        let (ca, cb) = (rects[a].center(), rects[b].center());
        ca.y.total_cmp(&cb.y).then(ca.x.total_cmp(&cb.x))
    });
    let mut rank = vec![0; rects.len()];
    for (new, &old) in order.iter().enumerate() {
        rank[old] = new;
    }
    let sorted = order.iter().map(|&i| rects[i]).collect();
    (sorted, group.into_iter().map(|g| rank[g]).collect())
}
