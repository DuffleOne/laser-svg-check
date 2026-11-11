# How it works

Laser Check answers one question about an SVG: is there anywhere the laser would trace the same line more than once? Then it can rewrite the file so it doesn't.

## 1. Reading the file

The SVG is parsed as XML ([`src/geom.rs`](../src/geom.rs)). Laser Check walks every element that draws something:

| Element | Becomes |
| --- | --- |
| `path` | its `d` data, arcs converted to curves |
| `polygon`, `polyline` | straight segments (closed for `polygon`) |
| `line` | one segment |
| `rect` | four lines, or lines and curves if it has rounded corners |
| `circle`, `ellipse` | four curves |

Each element's `transform`, and those of every group above it, are applied, so everything ends up in the same coordinate space. Nested `<svg>` elements are positioned by their `x` and `y`.

Things that don't draw are skipped: `defs`, `clipPath`, `mask`, `symbol`, `pattern`, `marker`, gradients, `style`, `title` and `metadata`. So is anything with `display="none"`.

Shapes with neither a stroke nor a fill are skipped too. Illustrator sometimes leaves invisible copies of artwork behind (`fill="none"` and no stroke), and laser software like LightBurn never imports them, so they aren't cut. Treating them as real lines would make the fix keep the invisible copy and delete the one that actually gets cut. Fill and stroke are worked out the way a browser would: inherited from groups, then presentation attributes, then `<style>` class rules, then the `style` attribute. `text`, `image` and `use` elements are counted and reported but not checked; laser files from Illustrator rarely rely on them for cut lines.

Units come from the root element. With `width="1000mm"` and a `viewBox` 2834.65 units wide, one unit is 0.353 mm. When the width has no unit (or `px`), Laser Check follows Illustrator and treats one unit as one point, 1/72 inch.

Every segment remembers which element it came from, its subpath, and whether that subpath was closed. That's what lets the fix put a file back together later.

## 2. Flattening

Curves are split into short straight pieces ([`src/analyze.rs`](../src/analyze.rs)). The number of pieces comes from the curve's second derivative, so the pieces never stray more than a quarter of the tolerance from the real curve. Long straight lines are also chopped up so no piece is longer than about 2 mm. Each piece keeps the curve parameter and the distance along its segment at both ends.

## 3. Finding doubled stretches

Pieces go into a spatial hash with 2 mm cells. For every pair of pieces from different segments that share a cell:

1. If they point in clearly different directions (more than 25° apart), skip them. They can cross but not run together. Opposite directions count as the same direction, since a laser cuts a line just as well backwards.
2. Otherwise, work out the stretch of the first piece that has the second within the tolerance (0.05 mm by default). This is a little linear algebra: project the second piece onto the first, and keep the part where the sideways distance is within the tolerance and the projection lands on the first piece.

Each pair is only handled in one cell, so nothing is counted twice.

The result is, for every segment, a list of stretches that something else lies along.

## 4. Deciding what matters

Measuring along single segments would get this wrong in both directions. A long doubled outline is often hundreds of tiny segments, each only a fraction of a millimetre. Meanwhile, the two sides of a sharp spike are within 0.05 mm of each other for a short way near the tip.

So the stretches are joined up along each whole subpath, wrapping around the start of closed shapes, and a joined run only counts once it is at least 2 mm long. Tips, tangent curves and near misses fall away. Shared edges, duplicated shapes and stray fragments drawn twice stay.

A file is flagged when the fix would remove at least 2 mm of line in total.

## 5. Fixing

The rule is simple: in drawing order, the first copy of a line wins. For each segment, the stretches covered by an earlier segment are marked for removal, but only where they fall inside a run that counted in step 4. Removed stretches closer together than the tolerance are merged, so no slivers are left behind.

Then each touched element is rebuilt ([`src/fix.rs`](../src/fix.rs)):

- Segments that keep their full length are kept as they are. Curves stay curves.
- Partly removed segments are cut with de Casteljau subdivision at the right curve parameters, which are found from the distances recorded when flattening.
- What survives is joined back into continuous runs. A gap only breaks a run where something was actually removed, and on a closed shape the run that reaches the end carries on into the one at the start, so the laser doesn't start and stop more than it has to.
- Coordinates are mapped back through the inverse of the element's transforms, so the element keeps its original `transform` attribute.

The rebuilt element is written as a `<path>` carrying all the original's attributes except the geometry ones (`d`, `points`, `x`, `cx`, `r` and so on). An element with nothing left is deleted, along with the whitespace before it. Every other byte of the file is copied unchanged.

## 6. Checking the fix

`laser-check verify original.svg original-fixed.svg` samples points along every line in the original, at intervals smaller than the tolerance, and checks each one still has a line within twice the tolerance in the fixed file. On real Illustrator exports with tens of thousands of segments, that's millions of points, and none go missing. CI runs the same check on the honeycomb example.

## Watching folders

The watcher ([`src/watch.rs`](../src/watch.rs)) uses FSEvents and sleeps until something changes. When an SVG appears it waits until the file has stopped growing for 1.5 seconds, so AirDrop and browser downloads have finished.

Each file is hashed with SHA-256. The hash is looked up in `~/Library/Application Support/laser-check/seen.json`:

- **never seen**: check it;
- **seen and clean**: skip it for good;
- **warned about**: skip it until `rewarn_days` have passed, then check and warn again.

Fixed copies are recorded as clean the moment they're written, so the watcher never trips over its own output. Entries untouched for 90 days are forgotten.

The checking itself runs in a short-lived child process (`laser-check scan …`). Parsing a large file can take tens of megabytes, and memory allocators rarely give that back, so keeping it out of the long-running watcher holds it at about 3 MB.

If anything needs your attention, the watcher asks macOS to open the app in alert mode. The window only exists while there's something to show.
