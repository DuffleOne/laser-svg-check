//! Checks a fix didn't lose anything: every point along the original lines
//! must still have a line within tolerance in the fixed file.

use std::collections::HashMap;

use kurbo::{Line, ParamCurveNearest, Point, Rect};

use crate::analyze::Analysis;
use crate::config::Settings;

/// Number of sampled points on `orig` with no line of `fixed` nearby.
pub fn missing(orig: &Analysis, fixed: &Analysis, s: &Settings) -> usize {
    let tol = 2.0 * s.tolerance_mm / orig.doc.mm_per_unit;
    let cell = 1.0 / orig.doc.mm_per_unit;
    let key = |p: Point| ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<Line>> = HashMap::new();
    for f in &fixed.flat {
        for w in f.pts.windows(2) {
            let r = Rect::from_points(w[0], w[1]).inflate(tol, tol);
            let (x0, y0) = key(Point::new(r.x0, r.y0));
            let (x1, y1) = key(Point::new(r.x1, r.y1));
            for x in x0..=x1 {
                for y in y0..=y1 {
                    grid.entry((x, y)).or_default().push(Line::new(w[0], w[1]));
                }
            }
        }
    }
    let mut miss = 0;
    for f in &orig.flat {
        for w in f.pts.windows(2) {
            let steps = ((w[1] - w[0]).hypot() / (tol * 2.0)).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let p = w[0].lerp(w[1], k as f64 / steps as f64);
                let near = grid
                    .get(&key(p))
                    .is_some_and(|v| v.iter().any(|l| l.nearest(p, 1e-9).distance_sq <= tol * tol));
                if !near {
                    miss += 1;
                }
            }
        }
    }
    miss
}
