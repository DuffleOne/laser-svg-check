mod agent;
mod analyze;
mod config;
mod fix;
mod geom;
mod render;
mod store;
mod ui;
mod verify;
mod watch;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use config::{Config, Settings};

const USAGE: &str = "\
Laser Check: finds lines in an SVG that a laser would cut more than once.

usage:
  laser-check [files or folders...]        open the review window
  laser-check check <files or folders...>  report doubled lines (exit 1 if any)
  laser-check fix <files or folders...>    write name-fixed.svg next to each problem file
  laser-check verify <a.svg> <a-fixed.svg> confirm a fix kept every line
  laser-check preview <in.svg> <out.png> [--area N] [--light]
                                           render the highlights to a PNG
  laser-check watch                        watch folders (what the login agent runs)
  laser-check uninstall                    stop the watcher and remove its login agent
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::load();
    let s = cfg.settings();
    let paths = |from: usize| {
        args[from.min(args.len())..]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    };
    match args.first().map(String::as_str) {
        Some("check") => cli_check(paths(1), s, false),
        Some("fix") => cli_check(paths(1), s, true),
        Some("verify") if args.len() == 3 => {
            let missing = verify::missing(
                &analyse_file(Path::new(&args[1]), &s)?,
                &analyse_file(Path::new(&args[2]), &s)?,
                &s,
            );
            if missing == 0 {
                println!("every line in {} is still in {}", args[1], args[2]);
                Ok(())
            } else {
                println!("{missing} sampled points have no line nearby in {}", args[2]);
                std::process::exit(1)
            }
        }
        Some("preview") if args.len() >= 3 => {
            let an = analyse_file(Path::new(&args[1]), &s)?;
            let area = args
                .iter()
                .position(|a| a == "--area")
                .and_then(|i| args.get(i + 1)?.parse::<usize>().ok());
            let focus = area
                .and_then(|i| an.areas.get(i.saturating_sub(1)))
                .map(|a| a.rect);
            let region = match focus {
                Some(r) => r.inflate(r.width().max(20.0), r.height().max(20.0)),
                None => an.doc.view,
            };
            let light = args.iter().any(|a| a == "--light");
            let pm = render::render(
                &an,
                &render::View {
                    region,
                    width: 2000,
                    height: 1400,
                    focus,
                },
                !light,
            );
            pm.save_png(&args[2])?;
            Ok(())
        }
        Some("watch") => watch::run(),
        Some("scan") => {
            watch::scan(&paths(1));
            Ok(())
        }
        Some("alert") => {
            ui::run(ui::Start::Alert(paths(1)));
            Ok(())
        }
        // Used by scripts/install-app.sh.
        Some("install-agent") => Ok(agent::install(&std::env::current_exe()?)?),
        Some("uninstall") => {
            agent::uninstall()?;
            println!("watcher stopped and removed; drag Laser Check to the Bin to finish");
            Ok(())
        }
        Some("help" | "-h" | "--help") => {
            print!("{USAGE}\nconfig: {}\n", Config::path().display());
            Ok(())
        }
        _ => {
            agent::sync(&cfg);
            // Finder may pass a -psn_ argument on old systems; ignore anything that isn't a path.
            ui::run(ui::Start::Review(
                args.iter()
                    .filter(|a| !a.starts_with('-'))
                    .map(PathBuf::from)
                    .collect(),
            ));
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
        && !p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn cli_check(paths: Vec<PathBuf>, s: Settings, fix: bool) -> Result<()> {
    let files = collect_svgs(paths);
    if files.is_empty() {
        eprintln!("no SVGs given\n\n{USAGE}");
        std::process::exit(2);
    }
    let mut bad = 0;
    for path in files {
        match analyse_file(&path, &s) {
            Ok(an) if an.has_problems(&s) => {
                bad += 1;
                println!(
                    "{}: {} cut twice, in {}",
                    path.display(),
                    ui::length(an.removed_mm),
                    ui::areas(an.areas.len())
                );
                if fix {
                    let out = fix::write_fixed(&an, &path)?;
                    store::mark_clean(&out);
                    println!("  fixed copy: {}", out.display());
                }
            }
            Ok(an) => {
                let note = if an.doc.skipped.is_empty() {
                    String::new()
                } else {
                    format!(" (ignored {})", an.doc.skipped.join(", "))
                };
                println!("{}: ok{note}", path.display());
            }
            Err(e) => {
                bad += 1;
                println!("{}: couldn't read: {e:#}", path.display());
            }
        }
    }
    if bad > 0 && !fix {
        std::process::exit(1);
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
        analyze::run(
            geom::parse(src).unwrap(),
            &Settings {
                tolerance_mm: 0.05,
                min_overlap_mm: 2.0,
            },
        )
    }

    fn s() -> Settings {
        Settings {
            tolerance_mm: 0.05,
            min_overlap_mm: 2.0,
        }
    }

    #[test]
    fn exact_duplicate_is_flagged_and_fixed() {
        let an = run(
            r#"<rect x="10" y="10" width="50" height="30"/><rect x="10" y="10" width="50" height="30"/>"#,
        );
        assert!(an.has_problems(&s()));
        let fixed = fix::fixed_svg(&an);
        assert_eq!(fixed.matches("<rect").count(), 1, "{fixed}");
        assert!(!run_doc(fixed).has_problems(&s()));
    }

    #[test]
    fn shared_edge_between_shapes_is_flagged() {
        // Two squares sharing the x=50 edge, like a honeycomb.
        let an = run(
            r#"<rect x="10" y="10" width="40" height="40"/><rect x="50" y="10" width="40" height="40"/>"#,
        );
        assert!(an.has_problems(&s()));
        let fixed = run_doc(fix::fixed_svg(&an));
        assert!(!fixed.has_problems(&s()));
    }

    #[test]
    fn crossing_lines_are_fine() {
        let an = run(
            r#"<line x1="0" y1="0" x2="100" y2="100"/><line x1="0" y1="100" x2="100" y2="0"/><line x1="0" y1="50" x2="100" y2="52"/>"#,
        );
        assert!(!an.has_problems(&s()), "{} mm", an.removed_mm);
    }

    #[test]
    fn touching_circles_are_fine() {
        let an = run(r#"<circle cx="50" cy="50" r="20"/><circle cx="90" cy="50" r="20"/>"#);
        assert!(!an.has_problems(&s()), "{} mm", an.removed_mm);
    }

    #[test]
    fn transformed_duplicate_is_found() {
        let an = run(
            r#"<path d="M10,10 C40,0 60,40 90,20"/><g transform="translate(5,5)"><path d="M5,5 C35,-5 55,35 85,15"/></g>"#,
        );
        assert!(an.has_problems(&s()));
        assert!(!run_doc(fix::fixed_svg(&an)).has_problems(&s()));
    }

    #[test]
    fn invisible_copy_is_ignored_and_visible_copy_kept() {
        // Illustrator leaves unstroked, unfilled copies that laser software
        // never sees. The visible copy must survive.
        let shape = "M10,10 L90,10 L90,60 L10,60 Z";
        let an = run(&format!(
            r##"<g><path d="{shape}" fill="none"/></g><path d="{shape}" fill="none" stroke="#000"/>"##
        ));
        assert!(!an.has_problems(&s()), "{} mm", an.removed_mm);
        assert_eq!(an.doc.skipped, vec!["1 invisible shape".to_string()]);
    }

    #[test]
    fn stroke_from_a_stylesheet_class_counts() {
        let an = run(r#"<style>.cls-1{fill:none;stroke:#000;} .ghost{fill:none}</style>
               <rect class="cls-1" x="10" y="10" width="50" height="30"/>
               <rect class="cls-1" x="10" y="10" width="50" height="30"/>
               <rect class="ghost" x="10" y="10" width="50" height="30"/>"#);
        assert!(an.has_problems(&s()));
        let fixed = fix::fixed_svg(&an);
        assert!(
            fixed.contains(r#"<rect class="cls-1""#),
            "first visible copy kept: {fixed}"
        );
    }

    #[test]
    fn areas_are_in_reading_order_and_add_up() {
        // Two doubled squares, the lower one on the left.
        let sq = |x: f64, y: f64| {
            format!(r##"<rect x="{x}" y="{y}" width="20" height="20" fill="none" stroke="#000"/>"##)
        };
        let body = [
            sq(150.0, 20.0),
            sq(150.0, 20.0),
            sq(20.0, 150.0),
            sq(20.0, 150.0),
            sq(150.0, 150.0),
            sq(150.0, 150.0),
        ]
        .concat();
        let an = run(&body);
        assert_eq!(an.areas.len(), 3);
        let c: Vec<_> = an.areas.iter().map(|a| a.rect.center()).collect();
        assert!(c[0].y < c[1].y, "top first");
        assert!(c[1].x < c[2].x, "then left to right");
        let sum: f64 = an.areas.iter().map(|a| a.mm).sum();
        assert!((sum - an.removed_mm).abs() < 1e-6, "{sum} vs {}", an.removed_mm);
        // Each square is 80 units round, about 28 mm.
        assert!(
            an.areas.iter().all(|a| (a.mm - 80.0 * 25.4 / 72.0).abs() < 0.5),
            "{:?}",
            an.areas
        );
    }

    fn run_doc(src: String) -> analyze::Analysis {
        analyze::run(geom::parse(src).unwrap(), &s())
    }
}
