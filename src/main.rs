mod analyze;
mod config;
mod fix;
mod geom;
mod render;
mod store;
mod ui;
mod watch;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use config::{Config, Settings};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::load();
    match args.first().map(String::as_str) {
        Some("check") => cli_check(&args[1..], cfg.settings(), false),
        Some("fix") => cli_check(&args[1..], cfg.settings(), true),
        // Debug helper: every original line must still have a fixed line on it.
        Some("verify") => {
            let s = cfg.settings();
            let a = analyse_file(Path::new(&args[1]), &s)?;
            let b = analyse_file(Path::new(&args[2]), &s)?;
            let tol = 2.0 * s.tolerance_mm / a.doc.mm_per_unit;
            let cell = 1.0 / a.doc.mm_per_unit;
            let mut grid: std::collections::HashMap<(i32, i32), Vec<(kurbo::Point, kurbo::Point)>> = Default::default();
            for f in &b.flat {
                for w in f.pts.windows(2) {
                    let r = kurbo::Rect::from_points(w[0], w[1]).inflate(tol, tol);
                    for x in (r.x0 / cell).floor() as i32..=(r.x1 / cell).floor() as i32 {
                        for y in (r.y0 / cell).floor() as i32..=(r.y1 / cell).floor() as i32 {
                            grid.entry((x, y)).or_default().push((w[0], w[1]));
                        }
                    }
                }
            }
            let (mut n, mut miss) = (0, 0);
            for f in &a.flat {
                for w in f.pts.windows(2) {
                    let steps = ((w[1] - w[0]).hypot() / (tol * 2.0)).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let p = w[0].lerp(w[1], k as f64 / steps as f64);
                        n += 1;
                        let key = ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32);
                        let hit = grid.get(&key).is_some_and(|v| v.iter().any(|&(q0, q1)| {
                            let l = kurbo::Line::new(q0, q1);
                            use kurbo::ParamCurveNearest;
                            l.nearest(p, 1e-9).distance_sq <= tol * tol
                        }));
                        if !hit {
                            miss += 1;
                            if miss < 5 { println!("missing near {p:?}"); }
                        }
                    }
                }
            }
            println!("{n} samples, {miss} with no fixed line nearby");
            Ok(())
        }
        // Debug helper: render <svg> <out.png> [area index]
        Some("render") => {
            let an = analyse_file(Path::new(&args[1]), &cfg.settings())?;
            let focus = args.get(3).and_then(|i| an.areas.get(i.parse::<usize>().ok()?)).copied();
            let region = focus.map(|r| r.inflate(r.width().max(20.0), r.height().max(20.0))).unwrap_or(an.doc.view);
            let pm = render::render(&an, &render::View { region, width: 2000, height: 1400, focus }, false);
            pm.save_png(&args[2])?;
            Ok(())
        }
        Some("watch") => watch::run(),
        Some("alert") => {
            ui::run(ui::Start::Alert(args[1..].iter().map(PathBuf::from).collect()));
            Ok(())
        }
        Some("help" | "-h" | "--help") => {
            eprintln!(
                "usage:\n  laser-check [files or folders...]   open the review window\n  laser-check watch                    watch folders in the background\n  laser-check check|fix <paths...>     check or fix from the terminal\n\nconfig: {}",
                Config::path().display()
            );
            Ok(())
        }
        _ => {
            // Finder may pass a -psn_ argument on old systems; ignore anything that isn't a path.
            let paths = args.iter().filter(|a| !a.starts_with('-')).map(PathBuf::from).collect();
            ui::run(ui::Start::Review(paths));
            Ok(())
        }
    }
}

pub fn analyse_file(path: &Path, s: &Settings) -> Result<analyze::Analysis> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let src = String::from_utf8_lossy(&bytes).into_owned();
    let doc = geom::parse(src)?;
    Ok(analyze::run(doc, s))
}

/// Expands folders (one level, plus nested folders) into the SVGs inside them.
pub fn collect_svgs(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut kids: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                kids.sort();
                let kids: Vec<PathBuf> = kids.into_iter().filter(|k| k.is_dir() || is_svg(k)).collect();
                out.extend(collect_svgs(kids));
            }
        } else if is_svg(&p) {
            out.push(p);
        }
    }
    out
}

pub fn is_svg(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn cli_check(args: &[String], s: Settings, fix: bool) -> Result<()> {
    for path in collect_svgs(args.iter().map(PathBuf::from).collect()) {
        let t = std::time::Instant::now();
        match analyse_file(&path, &s) {
            Ok(an) => {
                println!(
                    "{}: {} segs, {} doubled patches, {} areas, {:.1} mm cut twice{}{} ({:?})",
                    path.display(),
                    an.doc.segs.len(),
                    an.patches.len(),
                    an.areas.len(),
                    an.removed_mm,
                    if an.has_problems(&s) { "  << PROBLEM" } else { "" },
                    if an.doc.skipped.is_empty() { String::new() } else { format!(", skipped {}", an.doc.skipped.join(", ")) },
                    t.elapsed()
                );
                if fix && an.has_problems(&s) {
                    let out = fix::write_fixed(&an, &path)?;
                    println!("  wrote {}", out.display());
                }
            }
            Err(e) => println!("{}: error: {e:#}", path.display()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str) -> analyze::Analysis {
        let src = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 283.46 283.46">{body}</svg>"#
        );
        analyze::run(geom::parse(src).unwrap(), &Settings { tolerance_mm: 0.05, min_overlap_mm: 2.0 })
    }

    fn s() -> Settings {
        Settings { tolerance_mm: 0.05, min_overlap_mm: 2.0 }
    }

    #[test]
    fn exact_duplicate_is_flagged_and_fixed() {
        let an = run(r#"<rect x="10" y="10" width="50" height="30"/><rect x="10" y="10" width="50" height="30"/>"#);
        assert!(an.has_problems(&s()));
        let fixed = fix::fixed_svg(&an);
        assert_eq!(fixed.matches("<rect").count(), 1, "{fixed}");
        assert!(!run_doc(fixed).has_problems(&s()));
    }

    #[test]
    fn shared_edge_between_shapes_is_flagged() {
        // Two squares sharing the x=50 edge, like a honeycomb.
        let an = run(r#"<rect x="10" y="10" width="40" height="40"/><rect x="50" y="10" width="40" height="40"/>"#);
        assert!(an.has_problems(&s()));
        let fixed = run_doc(fix::fixed_svg(&an));
        assert!(!fixed.has_problems(&s()));
    }

    #[test]
    fn crossing_lines_are_fine() {
        let an = run(r#"<line x1="0" y1="0" x2="100" y2="100"/><line x1="0" y1="100" x2="100" y2="0"/><line x1="0" y1="50" x2="100" y2="52"/>"#);
        assert!(!an.has_problems(&s()), "{} mm", an.removed_mm);
    }

    #[test]
    fn touching_circles_are_fine() {
        let an = run(r#"<circle cx="50" cy="50" r="20"/><circle cx="90" cy="50" r="20"/>"#);
        assert!(!an.has_problems(&s()), "{} mm", an.removed_mm);
    }

    #[test]
    fn transformed_duplicate_is_found() {
        let an = run(r#"<path d="M10,10 C40,0 60,40 90,20"/><g transform="translate(5,5)"><path d="M5,5 C35,-5 55,35 85,15"/></g>"#);
        assert!(an.has_problems(&s()));
        assert!(!run_doc(fix::fixed_svg(&an)).has_problems(&s()));
    }

    fn run_doc(src: String) -> analyze::Analysis {
        analyze::run(geom::parse(src).unwrap(), &s())
    }
}
