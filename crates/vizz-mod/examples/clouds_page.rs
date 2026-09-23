//! Write the catalogue page for the site.
//!
//! ```sh
//! cargo run --example clouds_page -p vizz-mod > site/clouds/index.html
//! ```
//!
//! The page is generated rather than written because most of it is a
//! list of eighty-one things that already exist as data, and a
//! hand-kept copy of a list like that is wrong within a month. The
//! plates beside each entry come from `vizz-render`'s `plates`
//! example, which draws them with the shipping code; between the two,
//! nothing in the catalogue is a description of vizz written somewhere
//! else.
//!
//! Below the catalogue the page carries the provenance of everything
//! that is *not* a cloud — the shader's own shapes, the pattern stack,
//! colour, light, the audio analysis, the passes at the end. That half
//! is prose held in this file, because it is not a list the app keeps.
//! It lives here rather than on a page of its own so that "where did
//! this come from" has one answer and one URL.
//!
//! No scripts: the site is served with a content-security policy that
//! allows none, deliberately, and a catalogue is a thing to read.

use vizz_mod::generators::{Generator, Group, Kind, CATALOGUE, SIMULATIONS};

fn main() {
    let mut page = String::new();
    head(&mut page);
    intro(&mut page);
    for group in Group::ALL {
        let entries: Vec<&Generator> =
            CATALOGUE.iter().filter(|g| g.group == *group).collect();
        if entries.is_empty() {
            continue;
        }
        section(&mut page, *group, &entries, "gen");
    }
    live(&mut page);
    for group in Group::ALL {
        let entries: Vec<&Generator> =
            SIMULATIONS.iter().filter(|g| g.group == *group).collect();
        if entries.is_empty() {
            continue;
        }
        section(&mut page, *group, &entries, "sim");
    }
    engine(&mut page);
    sources(&mut page);
    tail(&mut page);
    print!("{page}");
}

/// What each group is, said once at the top of it.
fn blurb(group: Group, live: bool) -> &'static str {
    match (group, live) {
        (Group::Flow, _) => "Differential equations, integrated from one point on the attractor and recorded in time order. Consecutive points are consecutive moments, which is what makes the cloud crawl along itself rather than shimmer.",
        (Group::Map, _) => "Iterated rather than integrated: a rule applied over and over to a point in a plane, lifted into depth by delay embedding — Takens' theorem says a delayed coordinate unfolds the dynamics rather than merely decorating them.",
        (Group::Searched, _) => "Nobody chose these. Thirty coefficients are drawn at random and the result is kept only if it is chaotic, which about one draw in a few hundred is. The seed is the whole parameter, so the same seed is the same attractor on every machine.",
        (Group::Surface, _) => "Swept in scan order over two parameters, so the cloud reads as a surface rather than a fog.",
        (Group::Curve, _) => "Traced along a curve and thickened into a tube, in order, so the crawl runs along the curve.",
        (Group::Fractal, _) => "Self-similar at every scale, found by the chaos game, by escape time, or by marching rays inward until they stop escaping.",
        (Group::Grown, _) => "Built step by step by a rule, rather than evaluated: rewriting an alphabet, or letting particles wander in and stick.",
        (Group::Pattern, _) => "Sampled where a pattern is, rather than swept: a point is kept if the field says so.",
        (Group::Field, _) => "A grid of numbers stepped forward sixty times a second. The cloud is what rides it.",
        (Group::Bodies, _) => "Many things, each with a position of its own, and rules about how they see each other.",
    }
}

fn section(page: &mut String, group: Group, entries: &[&Generator], scope: &str) {
    page.push_str(&format!(
        "\n<h2 id=\"{}\">{}</h2>\n<p class=\"lede\">{}</p>\n<div class=\"plates\">\n",
        anchor(group, scope),
        escape(group.label()),
        blurb(group, scope == "sim"),
    ));
    for g in entries {
        let plate = if scope == "sim" {
            format!("/img/clouds/sim-{}.webp", g.id)
        } else {
            format!("/img/clouds/{}.webp", g.id)
        };
        page.push_str("  <figure class=\"plate\">\n");
        page.push_str(&format!(
            "    <img src=\"{}\" width=\"600\" height=\"400\" loading=\"lazy\" decoding=\"async\" alt=\"{} — a point cloud\">\n",
            plate,
            escape(g.name),
        ));
        page.push_str("    <figcaption>\n");
        page.push_str(&format!(
            "      <h3>{}</h3>\n      <p class=\"spec\"><code>{}:{}</code></p>\n",
            escape(g.name),
            scope,
            g.id
        ));
        page.push_str(&format!("      <p>{}</p>\n", escape(g.about)));
        if !g.params.is_empty() {
            page.push_str("      <p class=\"knobs\"><span>knobs</span> ");
            let knobs: Vec<String> = g
                .params
                .iter()
                .map(|p| {
                    let range = match p.kind {
                        Kind::Number { min, max } => format!("{} to {}", trim(min), trim(max)),
                        Kind::Seed => "any whole number".to_string(),
                        Kind::Text => "text".to_string(),
                    };
                    format!(
                        "<code>{}</code> {} <span class=\"dim\">({}, default {})</span>",
                        escape(p.key),
                        escape(p.about),
                        range,
                        escape(p.default)
                    )
                })
                .collect();
            page.push_str(&knobs.join("<br>\n      "));
            page.push_str("</p>\n");
        }
        let cite = escape(g.cite);
        if g.link.is_empty() {
            page.push_str(&format!("      <p class=\"cite\">{cite}</p>\n"));
        } else {
            page.push_str(&format!(
                "      <p class=\"cite\"><a href=\"{}\">{cite}</a></p>\n",
                escape(g.link)
            ));
        }
        page.push_str("    </figcaption>\n  </figure>\n");
    }
    page.push_str("</div>\n");
}

fn anchor(group: Group, scope: &str) -> String {
    format!("{scope}-{}", group.label().replace(' ', "-"))
}

/// A number as a person would write it: no trailing zeros after a
/// decimal point, and no stripping from a whole one — 180 is not 18,
/// and 0 is not nothing.
fn trim(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn head(page: &mut String) {
    page.push_str(r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>vizz — clouds from equations</title>
<link rel="icon" type="image/png" href="/img/icon.png">
<link rel="apple-touch-icon" href="/img/icon.png">
<meta name="description" content="Every point cloud vizz can make from an equation, every simulation it can run live, and everything else a frame is made of: what each one is, what it comes from, and who found it.">
<meta property="og:title" content="vizz — clouds from equations">
<meta property="og:description" content="Sixty-six generators, fifteen live simulations, and the rest of the engine, with the paper each one comes from.">
<meta property="og:type" content="article">
<style>
  /* One file, no build step, no CDN — the same rule as the rest of the
     site, and no scripts at all: the page is a catalogue, and the
     server's content-security policy allows none. The tokens are
     copied from the docs page rather than shared, because two static
     files cannot import from each other without a build. */
  :root {
    --bg: #0a0c12;
    --panel: #12151d;
    --line: #232838;
    --text: #e8ecf5;
    --dim: #97a0b5;
    --accent: #7fb5ff;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    background: var(--bg);
    color: var(--text);
    font: 16px/1.65 ui-sans-serif, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  a { color: var(--accent); }
  code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: .92em;
    background: #171b26; padding: .1em .35em; border-radius: 4px;
  }
  .dim { color: var(--dim); }

  .topbar {
    position: sticky; top: 0; z-index: 20;
    background: rgba(10,12,18,.92);
    backdrop-filter: blur(8px);
    border-bottom: 1px solid var(--line);
    padding: 10px 24px;
    display: flex; align-items: center; gap: 16px;
  }
  .topbar img { width: 22px; height: 22px; border-radius: 5px; }
  .topbar strong { letter-spacing: -.02em; }
  .topbar .sp { flex: 1; }
  .topbar a { text-decoration: none; }

  main { max-width: 1180px; margin: 0 auto; padding: 32px 24px 80px; }
  h1 { font-size: clamp(30px, 5vw, 44px); line-height: 1.1; letter-spacing: -.02em; margin: 8px 0 12px; }
  h2 {
    font-size: 24px; letter-spacing: -.01em; margin: 56px 0 6px;
    padding-top: 20px; border-top: 1px solid var(--line);
  }
  h3 { font-size: 17px; margin: 0 0 2px; letter-spacing: -.01em; }
  p { margin: 10px 0; }
  .lede { color: var(--dim); max-width: 64ch; }
  .intro { max-width: 68ch; }

  /* The index: eleven links, so no sidebar is needed. */
  .jump { display: flex; flex-wrap: wrap; gap: 8px 10px; margin: 20px 0 8px; padding: 0; list-style: none; }
  .jump a {
    display: inline-block; text-decoration: none;
    border: 1px solid var(--line); border-radius: 999px;
    padding: 4px 12px; font-size: 14px; color: var(--text);
  }
  .jump a:hover { border-color: var(--accent); color: var(--accent); }

  .plates {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 18px; margin-top: 20px;
  }
  .plate {
    margin: 0; background: var(--panel);
    border: 1px solid var(--line); border-radius: 10px; overflow: hidden;
    display: flex; flex-direction: column;
  }
  .plate img {
    display: block; width: 100%; height: auto; aspect-ratio: 3 / 2;
    background: #05070b; border-bottom: 1px solid var(--line);
  }
  .plate figcaption { padding: 14px 16px 16px; font-size: 14.5px; }
  .plate p { margin: 6px 0 0; }
  .spec { margin-top: 4px !important; }
  .knobs { font-size: 13.5px; color: var(--dim); line-height: 1.5; }
  .knobs span:first-child {
    display: block; text-transform: uppercase; letter-spacing: .08em;
    font-size: 11px; margin-bottom: 3px;
  }
  .cite { font-size: 13px; color: var(--dim); border-top: 1px solid var(--line); padding-top: 8px; margin-top: 10px !important; }
  /* The prose half. A single readable column rather than the
     catalogue's grid: there is no picture to put beside these, and a
     paragraph set to the full width of a plate grid is unreadable. */
  .piece { max-width: 74ch; padding: 18px 0; border-top: 1px solid var(--line); }
  .piece h3 { margin-bottom: 4px; }
  .piece p { color: var(--dim); font-size: 15px; }
  .src {
    margin-top: 12px !important; font-size: 14px;
    padding-left: 12px; border-left: 2px solid var(--line);
  }
  .src b { color: var(--text); font-weight: 600; }
  /* Marking what has no outside source is as much this page's job as
     citing what has: an entry with a blank where the credit goes reads
     as a citation somebody forgot to write. */
  .src.ours { border-left-style: dashed; }

  .sources { max-width: 90ch; font-size: 14.5px; color: var(--dim); padding-left: 22px; }
  .sources li { margin: 7px 0; }
  .sources strong { color: var(--text); font-weight: 600; }

  footer { margin-top: 64px; padding-top: 20px; border-top: 1px solid var(--line); color: var(--dim); font-size: 14.5px; }
  @media (max-width: 640px) {
    main { padding: 24px 16px 64px; }
    .plates { grid-template-columns: 1fr; }
  }
</style>
</head>
<body>

<div class="topbar">
  <a href="/"><img src="/img/icon.png" alt=""></a>
  <a href="/"><strong>vizz</strong></a>
  <span class="dim">clouds</span>
  <span class="sp"></span>
  <a href="/docs">Docs</a>
  <a href="https://github.com/legofsalmon/vizz">GitHub</a>
</div>

<main>
"#);
}

fn intro(page: &mut String) {
    let generators = CATALOGUE.len();
    let simulations = SIMULATIONS.len();
    page.push_str(&format!(r##"<h1>Clouds from equations</h1>
<div class="intro">
<p class="lede">vizz draws point clouds. Most of them come from a scanner, a
camera or a file — and {generators} of them come from mathematics, with
{simulations} more that are still running while you watch.</p>

<p>Every picture on this page was drawn by the app's own code, from the same
functions that fill a slot when you pick one from the menu, and through the
same renderer that puts it on screen. Nothing here is an artist's impression
of what a generator makes.</p>

<p><strong>To use one:</strong> open the clouds section of the panel and pick from
<em>generate…</em>, or start vizz with <code>--cloud gen:thomas</code>. A made cloud
takes the next free slot and behaves exactly as a scanned one does: chosen by
name, crossed to with a scene change, captured by a preset, lit, and remade the
next time you launch. The ones with knobs take them after a <code>?</code> —
<code>gen:torus-knot?p=2;q=5</code>. The live ones run in the live slot:
<code>--live-cloud sim:fluid</code>, or <em>simulate…</em> beside <em>receive</em>.</p>

<p><strong>Where these came from.</strong> Almost none of it is ours. Each entry says
whose work it is, and links to somewhere to read about it where there is
somewhere worth sending you. If we have credited something wrongly, that is a
bug like any other — <a href="https://github.com/legofsalmon/vizz/issues">please
tell us</a>.</p>

<p><strong>And the rest of it.</strong> Clouds are the part of vizz with the
longest bibliography, but the shader's own shapes, the pattern stack, the
colour, the light, the beat detection and the passes at the end all came from
somewhere too. Those are <a href="#engine">below the catalogue</a>, and
everything on the page is <a href="#sources">in one list</a> at the foot of
it.</p>
</div>

<ul class="jump">
"##));
    for group in Group::ALL {
        if CATALOGUE.iter().any(|g| g.group == *group) {
            page.push_str(&format!(
                "  <li><a href=\"#{}\">{}</a></li>\n",
                anchor(*group, "gen"),
                escape(group.label())
            ));
        }
    }
    page.push_str(
        "  <li><a href=\"#alive\">alive</a></li>\n  <li><a href=\"#engine\">the rest</a></li>\n  <li><a href=\"#sources\">sources</a></li>\n</ul>\n",
    );
}

fn live(page: &mut String) {
    page.push_str(&format!(r#"
<h2 id="alive">{} that are alive</h2>
<p class="lede">These are not made once. Each runs on a thread of its own at the
frame rate and streams into the live slot, driven by whatever the audio input
is doing: the four bands, the loudness, and where the bar is. That is the whole
interface they get — deliberately, because the point is a <em>cloud</em> that
happens to be alive rather than a second engine with its own controls.</p>
<p class="lede">The plates below are each one caught a couple of seconds in, with
no audio playing. The ones that are about growing were given longer.</p>
"#, SIMULATIONS.len()));
}

fn sources(page: &mut String) {
    page.push_str(r#"
<h2 id="sources">Sources</h2>
<p class="lede">Every entry above, in one list, with whoever found it. Where the
original paper is behind a paywall the link goes to an encyclopaedia article
instead — the citation is the thing to keep.</p>

<h3>The clouds</h3>
<ol class="sources">
"#);
    let mut all: Vec<&Generator> = CATALOGUE.iter().chain(SIMULATIONS).collect();
    all.sort_by_key(|g| g.name.to_lowercase());
    for g in all {
        let cite = escape(g.cite);
        let body = if g.link.is_empty() {
            cite
        } else {
            format!("<a href=\"{}\">{cite}</a>", escape(g.link))
        };
        page.push_str(&format!(
            "  <li><strong>{}</strong> — {body}</li>\n",
            escape(g.name)
        ));
    }
    page.push_str("</ol>\n\n<h3>Everything else</h3>\n<ul class=\"sources\">\n");
    // In page order rather than alphabetical: these are seven groups of
    // related decisions, and sorting them by name would interleave the
    // beat detection with the optics for no reader's benefit.
    for part in ENGINE {
        for piece in part.pieces {
            page.push_str(&format!(
                "  <li><strong>{}</strong> — {}</li>\n",
                escape(piece.name),
                piece.source
            ));
        }
    }
    page.push_str("</ul>\n");
}

fn tail(page: &mut String) {
    page.push_str(r#"
<footer>
<p>This page is generated from the app's own catalogue and drawn with the app's
own code, so it cannot drift from what vizz actually ships. vizz is
<a href="https://github.com/legofsalmon/vizz">open source</a>, MIT licensed.</p>
</footer>

</main>
</body>
</html>
"#);
}

// --- The rest of the picture ------------------------------------------
//
// Everything below the catalogue: the parts of a vizz frame that are not
// clouds. These are prose rather than data, because unlike the
// eighty-one clouds they are not a list the app keeps — they are one-off
// decisions about how a frame gets made, and the thing worth recording
// about each is where it came from.
//
// They live here, beside the catalogue, so that the provenance of the
// whole app is one page. It was briefly two, and two pages covering the
// same ground diverge the moment one of them is easier to edit.

/// One system, and whoever it is from.
struct Piece {
    name: &'static str,
    /// What it is. Written as HTML and emitted as-is — these are
    /// paragraphs with links in them, not values out of the catalogue.
    about: &'static str,
    /// Who it is from, without a "Source:" label. The label belongs to
    /// the entry; the sources list at the foot of the page prints the
    /// same string under a heading that already says as much, and a
    /// label repeated there would read like a stutter.
    source: &'static str,
    /// Set where there is nothing outside vizz to cite. Marked rather
    /// than left blank: a system listed with an empty line beside it
    /// reads as a citation somebody forgot to write.
    ours: bool,
}

struct Part {
    id: &'static str,
    title: &'static str,
    lede: &'static str,
    pieces: &'static [Piece],
}

const ENGINE: &[Part] = &[
    Part {
        id: "shapes",
        title: "shapes",
        lede: "A filled slot is not the only way vizz gets a form on screen. These are what <code>/shape/mode</code> sweeps through, and most of them never touch memory at all: the particle's index is hashed into four numbers and the shape is a function of those four, evaluated in the vertex stage. That is why the sweep can morph one into the next — a particle keeps its hashes, so the same point travels between forms rather than the field being scattered again.",
        pieces: &[
            Piece {
                name: "Solid sphere and hollow shell",
                about: r#"<p>Both are sampled <em>uniformly by volume</em> rather than by angle, which is the difference between a sphere and a sphere with a bright clot at each pole. Height is drawn uniformly from −1 to 1 and the ring radius follows from it; for the solid sphere the radius is scaled by the cube root of a uniform draw, and for the shell it stays near the surface.</p>"#,
                source: r#"the uniform-height trick is <a href="https://mathworld.wolfram.com/ArchimedesHat-BoxTheorem.html">Archimedes' hat-box theorem</a> — equal slices of a sphere carry equal area — in the form usually written up as <a href="https://mathworld.wolfram.com/SpherePointPicking.html">sphere point picking</a>."#,
                ours: false,
            },
            Piece {
                name: "Trefoil knot",
                about: r#"<p>The simplest non-trivial knot, drawn as a core curve and thickened into a tube by scattering particles around it. vizz uses the standard <code>(2,&nbsp;3)</code> torus-knot parametrisation. The catalogue above has a knot of its own with <code>p</code> and <code>q</code> to turn; this one is fixed, because it is a stop on a sweep rather than a cloud you configure.</p>"#,
                source: r#"classical knot theory; the parametrisation is the standard one, as given on <a href="https://mathworld.wolfram.com/TrefoilKnot.html">MathWorld</a>."#,
                ours: false,
            },
            Piece {
                name: "Lorenz and Aizawa",
                about: r#"<p>The two attractors that predate the catalogue. Unlike the rest of the sweep these are not evaluated per vertex: both are integrated once on the CPU at startup — Lorenz with <code>σ&nbsp;10</code>, <code>ρ&nbsp;28</code>, <code>β&nbsp;8/3</code>, the classic values — the first 20,000 steps discarded as transient, and the next 65,536 stored in trajectory order in the first two cloud slots. Consecutive points are consecutive moments, so advancing every particle's index makes the cloud crawl <em>along</em> the attractor rather than shimmer in place. Aizawa is there as a contrast partner: rounder, shell-like, with a spike through the poles, so the morph between the two has somewhere to go. Both appear in the catalogue above as well, where they can be asked for by name into any slot.</p>"#,
                source: r#"Edward N. Lorenz, <a href="https://journals.ametsoc.org/view/journals/atsc/20/2/1520-0469_1963_020_0130_dnf_2_0_co_2.xml">“Deterministic Nonperiodic Flow”</a>, <em>Journal of the Atmospheric Sciences</em> 20(2), 1963. Aizawa is named for Yoji Aizawa, whose work on chaotic flows it comes out of; unlike Lorenz it has no single canonical paper behind it, and the parameter set vizz uses is the one that circulates in graphics, stated the same way in <a href="https://analogparadigm.com/downloads/alpaca_17.pdf">Analog Paradigm's write-up</a>."#,
                ours: false,
            },
            Piece {
                name: "Torus, grid plane, cloud pair",
                about: r#"<p>A torus from its two angles, a grid plane with a travelling ripple, and the cloud pair — which is not a shape at all but a blend between any two loaded slots: a scan, an attractor, a word, a live video frame. The sweep only reaches <em>adjacent</em> modes, so crossing a scan into an attractor needs a control of its own, which is what <code>/cloud/morph</code> is.</p>"#,
                source: "plain parametric forms with nothing behind them worth citing.",
                ours: true,
            },
            Piece {
                name: "Wind",
                about: r#"<p>A displacement laid over whatever shape is showing, so a still form leans and breathes without becoming a simulation. Two octaves of one field, each drifting at its own pace so the wind never settles into a pattern the eye can lock onto. It runs per vertex, on every form including a loaded scan, because the field it uses is six trigonometric terms — a curl of noise would not fit there.</p>"#,
                source: r#"the <a href="https://en.wikipedia.org/wiki/Arnold%E2%80%93Beltrami%E2%80%93Childress_flow">Arnold–Beltrami–Childress flow</a> with <code>A&nbsp;=&nbsp;B&nbsp;=&nbsp;C&nbsp;=&nbsp;1</code>: a steady solution of the Euler equations, divergence-free by construction, and chaotic in its streamlines — the standard example of a simple field that mixes."#,
                ours: false,
            },
        ],
    },
    Part {
        id: "layers",
        title: "layers",
        lede: "The print-look counterpart to the particle field, and the other half of what vizz puts on a screen.",
        pieces: &[
            Piece {
                name: "The stack",
                about: r#"<p>Eight procedural pattern layers — rings, stripes, checker, polygon, star, rays, dots — in flat ink colours on coloured paper, each with its own similarity transform, drawn in a single fullscreen pass and composited in-register.</p>"#,
                source: r#"555-5555's visuals for Caribou's live shows, which is the look this was built to reach. Not an algorithm and not a paper — an idiom, borrowed knowingly. The generators themselves are ordinary procedural patterns."#,
                ours: false,
            },
            Piece {
                name: "Moiré",
                about: r#"<p>The reason the stack has eight layers and not one. Two periodic patterns at close frequencies interfere into a third, much coarser pattern that neither layer contains, and it moves far faster than either layer does when you drift one of them. The built-in show is largely made of this.</p>"#,
                source: r#"Isaac Amidror, <a href="https://link.springer.com/book/10.1007/978-1-84882-181-1"><em>The Theory of the Moiré Phenomenon, Volume I: Periodic Layers</em></a>, Springer."#,
                ours: false,
            },
            Piece {
                name: "Blend modes",
                about: r#"<p>Multiply, screen, add, difference, exclusion and subtract, the vocabulary the look is built in.</p>"#,
                source: r#"the separable blend modes of <a href="https://www.w3.org/TR/compositing-1/">W3C Compositing and Blending Level&nbsp;1</a>. vizz departs from the spec in one deliberate way: it blends on sRGB-<em>encoded</em> values rather than linear ones, because compositing on encoded values is the print-era behaviour this aesthetic grew out of. The single conversion to linear happens on the way out of the shader."#,
                ours: false,
            },
            Piece {
                name: "Analytic antialiasing",
                about: r#"<p>Hard-edged patterns alias badly, and the usual fix — <code>fwidth</code> — lies at exactly the discontinuities these patterns are made of. Instead each layer carries one scalar for how many pattern units a pixel spans, propagated by the chain rule through the layer transform, and each generator turns a signed distance into coverage over that footprint. Past Nyquist the coverage converges to flat duty-cycle tone, so the moiré survives where the sparkle does not.</p>"#,
                source: r#"the technique Iñigo Quilez describes in <a href="https://iquilezles.org/articles/filterableprocedurals/">“filtering procedural textures”</a>. The per-generator gradient magnitudes are worked out for these generators specifically."#,
                ours: false,
            },
        ],
    },
    Part {
        id: "colour",
        title: "colour",
        lede: "What comes out of the shader, and what stops it turning to white.",
        pieces: &[
            Piece {
                name: "Cosine palettes",
                about: r#"<p>The four built-in gradients are <code>colour = a + b·cos(τ(c·t + d))</code>, four coefficients each. They are baked into a lookup texture on the CPU rather than evaluated in the shader, so a palette you load from a list of colour stops sits in the same bank and is indistinguishable from a built-in as far as the renderer is concerned.</p>"#,
                source: r#"Iñigo Quilez, <a href="https://iquilezles.org/articles/palettes/">“palettes”</a>."#,
                ours: false,
            },
            Piece {
                name: "HSV",
                about: r#"<p>Palette 0 stays procedural rather than baked, so <code>/particles/hue</code> keeps meaning a rotation through hue rather than an offset into a ramp.</p>"#,
                source: r#"Alvy Ray Smith, <a href="https://alvyray.com/Papers/CG/color78.pdf">“Color Gamut Transform Pairs”</a>, SIGGRAPH 1978 — the paper HSV comes from."#,
                ours: false,
            },
            Piece {
                name: "The tone shoulder",
                about: r#"<p>Glow and trails push values well past 1, and a hard clip turns highlights into flat white blobs. The composite pass ends with <code>c / (1 + 0.15c)</code>, which rolls the top off and leaves midtones essentially untouched. It is the default finish; the graded one below replaces it when asked.</p>"#,
                source: r#"the <code>c/(1+c)</code> curve from Reinhard, Stark, Shirley and Ferwerda, <a href="https://www.cs.utah.edu/docs/techreports/2002/pdf/UUCS-02-001.pdf">“Photographic Tone Reproduction for Digital Images”</a>, SIGGRAPH 2002. The <code>0.15</code> is tuned for this picture, not from the paper."#,
                ours: false,
            },
            Piece {
                name: "sRGB",
                about: r#"<p>The transfer function the vector stack encodes and decodes with, and the one the output textures are in.</p>"#,
                source: r#"<a href="https://en.wikipedia.org/wiki/SRGB">IEC&nbsp;61966-2-1</a>."#,
                ours: false,
            },
        ],
    },
    Part {
        id: "light",
        title: "light",
        lede: "A cloud of dots has no surface, which makes lighting one a question rather than a setting.",
        pieces: &[
            Piece {
                name: "Estimated normals",
                about: r#"<p>Most scanners export points with no surface direction, and the sun cannot light a surface it cannot orient. vizz reads normals from a PLY that carries them and works them out from each point's twelve nearest neighbours when it does not: fit a plane through the neighbourhood, and the plane's normal is the surface's — the eigenvector of the neighbourhood's covariance with the smallest eigenvalue.</p>"#,
                source: r#"Hoppe, DeRose, Duchamp, McDonald and Stuetzle, <a href="https://hhoppe.com/proj/recon/">“Surface Reconstruction from Unorganized Points”</a>, SIGGRAPH 1992, where the local plane fit is step one. vizz stops there: the paper goes on to propagate a consistent orientation across the cloud, and vizz instead flips each normal towards the eye, which is a better answer for a scan of a room seen from inside it."#,
                ours: false,
            },
            Piece {
                name: "Lamps and the sun",
                about: r#"<p>Two movable lamps that fall off with distance and work on anything with a position, and a directional sun that lights <em>surfaces</em> — the wall facing it comes up, the wall facing away goes down. A cloud with no normals is left alone by the sun rather than lit from a direction nobody measured.</p>"#,
                source: r#"<a href="https://en.wikipedia.org/wiki/Lambert%27s_cosine_law">Lambert's cosine law</a> (Johann Heinrich Lambert, <em>Photometria</em>, 1760) for the directional term. The distance falloff is a smooth <code>r²/(d²+r²)</code> rather than inverse-square, so there is no singularity to divide by on stage."#,
                ours: false,
            },
            Piece {
                name: "PLY",
                about: r#"<p>The format a scan arrives in, ASCII or binary little-endian, and also the wire format for live point-cloud streaming — there is no wrapper protocol, because a PLY header already states its own frame length. The simulations above reach the renderer down the same pipe.</p>"#,
                source: r#"<a href="https://en.wikipedia.org/wiki/PLY_(file_format)">the Stanford polygon format</a>, Greg Turk, Stanford University."#,
                ours: false,
            },
            Piece {
                name: "Depth of field",
                about: r#"<p>Done by resizing the sprite rather than blurring the frame: a defocused point light <em>is</em> a larger, dimmer disc, so the particle's billboard grows with its distance from the focus plane and dims by the square of that growth. Closer to the real thing than a post-process blur, and it costs nothing.</p>"#,
                source: "vizz, on the ordinary optics of the circle of confusion.",
                ours: true,
            },
        ],
    },
    Part {
        id: "randomness",
        title: "randomness",
        lede: "Where every particle that is not read out of a slot comes from.",
        pieces: &[
            Piece {
                name: "The particle hash",
                about: r#"<p>Every particle is derived from its index and nothing else — no state carried between frames, which is what makes the count free to change mid-set and the whole field scrubbable. Four independent random streams per particle come out of one integer mixer. It replaced a float hash whose repeats at high counts stacked particles on each other and pushed them into the tone-map shoulder; an integer mixer has no such cliff, and is stable across GPUs in a way anything built on <code>sin()</code> is not.</p>"#,
                source: r#"the <code>lowbias32</code> constants from Chris Wellons' <a href="https://github.com/skeeto/hash-prospector">hash prospector</a>, described in <a href="https://nullprogram.com/blog/2018/07/31/">“Prospecting for Hash Functions”</a>."#,
                ours: false,
            },
        ],
    },
    Part {
        id: "audio",
        title: "audio",
        lede: "The four bands, the loudness and the bar — the whole of what the simulations above are driven by, and most of what the rest of the app listens to.",
        pieces: &[
            Piece {
                name: "Window and spectrum",
                about: r#"<p>A 2048-point FFT at 48&nbsp;kHz — a 43&nbsp;ms window, 23&nbsp;Hz a bin — through a periodic Hann window, with the band magnitudes corrected for the window's power loss so that a band meter and a gain control mean something absolute rather than something relative to the window.</p>"#,
                source: r#"the transform itself is the <a href="https://github.com/ejmahler/RustFFT">rustfft</a> crate; the window and its correction factor are from Fredric J. Harris, <a href="https://ieeexplore.ieee.org/document/1455106/">“On the Use of Windows for Harmonic Analysis with the Discrete Fourier Transform”</a>, <em>Proceedings of the IEEE</em> 66(1), 1978, and Parseval's theorem for the normalisation."#,
                ours: false,
            },
            Piece {
                name: "Onset detection",
                about: r#"<p>Positive spectral flux: how much energy <em>appeared</em> since the last frame, summing only the bins that went up. Summing the drops as well would make a note ending look like a note starting.</p>"#,
                source: r#"the half-wave-rectified spectral flux detector, surveyed and compared in Simon Dixon, <a href="https://www.dafx.de/paper-archive/2006/papers/p_133.pdf">“Onset Detection Revisited”</a>, DAFx-06."#,
                ours: false,
            },
            Piece {
                name: "Tempo",
                about: r#"<p>Autocorrelation of the onset signal: if the music has a pulse, the onset signal correlates with itself at the beat period, and peak-picking the correlation gives a BPM without anything ever having to decide what a beat <em>is</em>. The characteristic failure is the octave error — 180 for a 90&nbsp;BPM track — because a signal that correlates at one period also correlates at its multiples. Two guards: a prior preferring the middle of the range, and a check for whether half the candidate explains the signal comparably well. A confidence figure comes out alongside, so ambient material with no pulse reports low confidence rather than a confident wrong answer.</p>"#,
                source: r#"autocorrelation of an onset-strength envelope, as set out in Daniel P. W. Ellis, <a href="https://www.ee.columbia.edu/~dpwe/pubs/Ellis07-beattrack.pdf">“Beat Tracking by Dynamic Programming”</a>, <em>Journal of New Music Research</em> 36(1), 2007. vizz uses the tempo-estimation half and not the dynamic-programming beat tracker — a VJ needs the rate, and the phase comes from the pads."#,
                ours: false,
            },
        ],
    },
    Part {
        id: "frame",
        title: "frame",
        lede: "Everything that happens to the picture after the clouds are in it.",
        pieces: &[
            Piece {
                name: "Feedback trails",
                about: r#"<p>The pass that makes the output look like VJ material. Last frame's result is zoomed and rotated a little and mixed back in with the new one, so motion smears into trails and a sustained zoom builds a tunnel out of whatever is on screen. A lerp rather than an accumulation, because adding the history outright is a geometric series that saturates to white within a second.</p>"#,
                source: r#"analogue video feedback, which is where this whole idiom comes from — Steina and Woody Vasulka's work and the <a href="https://en.wikipedia.org/wiki/Sandin_Image_Processor">Sandin Image Processor</a> (Dan Sandin, 1971–73). The implementation is an ordinary ping-pong of two <code>Rgba16Float</code> textures."#,
                ours: false,
            },
            Piece {
                name: "Kaleidoscope and lens",
                about: r#"<p>Mirror, quad mirror and a six-wedge kaleidoscope, all done as a fold of UV space before sampling; a radial RGB split that leaves green alone, so the frame fringes towards the edges the way a lens does instead of shifting hue; and a cheap six-tap bloom.</p>"#,
                source: "vizz. Standard screen-space constructions with no particular paper behind them.",
                ours: true,
            },
            Piece {
                name: "The graded finish",
                about: r#"<p>An alternative to the shoulder, faded in with <code>/fx/grade</code>. Three things change together. The exposure is metered: a histogram of log luminance over the lit pixels every frame, with the exposure eased towards putting the brightest one percent on the curve's shoulder. In the app the meter only ever darkens, so a look that clips is pulled back while a fade still reaches black. The glow becomes a mip chain: the frame halved six times and summed back up, so it falls off from a sharp core to a wide haze instead of showing its taps. And the curve is AgX, which compresses highlights over about sixteen stops in log space and desaturates as it goes, so a dense core of coloured sprites reads as a hot centre fading into its colour rather than a flat disc of clipped primary. The background is taken out before grading and put back after, so a colour chosen to match a room stays that colour. Every plate on this page is drawn through it, metered per plate.</p>"#,
                source: r#"the curve is Troy Sobotka's <a href="https://github.com/sobotka/AgX">AgX</a>, in the polynomial fit from Benjamin Wrensch's <a href="https://iolite-engine.com/blog_posts/minimal_agx_implementation">“Minimal AgX Implementation”</a> (2023). The bloom filters are from Jorge Jimenez, <a href="https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/">“Next Generation Post Processing in Call of Duty: Advanced Warfare”</a>, SIGGRAPH 2014: a thirteen-tap downsample, Karis-averaged on the first step, and a tent on the way up. Metering from a percentile of a log-luminance histogram is how game engines' auto-exposure works, <a href="https://dev.epicgames.com/documentation/en-us/unreal-engine/auto-exposure-in-unreal-engine">Unreal's</a> among them."#,
                ours: false,
            },
            Piece {
                name: "Gravity wells",
                about: r#"<p>Four attractors and repulsors bending the cloud from a layer above the scenes. Deliberately <em>not</em> a simulation: every particle here is a function of its index with no state between frames, and integrating velocities would throw that away for physics nobody is checking. The falloff is <code>r²/(d²+r²)</code> — one at the centre, a half at the radius, asymptotically nothing beyond — because a hard cutoff shows up as a visible shell in the cloud.</p>"#,
                source: "vizz. A displacement field that reads as gravity, not a gravity model.",
                ours: true,
            },
            Piece {
                name: "Camera moves",
                about: r#"<p>Ten paths — orbit, sway, push, pull, crane, spiral, look around, fly through, walkthrough and a slow drift — each returning a delta added to the camera the faders already describe, with rate in bars so a move locks to the same clock as the sequencer.</p>"#,
                source: "vizz. The shapes are borrowed from camera work rather than from anything written down.",
                ours: true,
            },
        ],
    },
];

fn engine(page: &mut String) {
    page.push_str(r#"
<h2 id="engine">The rest of the picture</h2>
<div class="intro">
<p class="lede">The clouds are the part of vizz with the longest bibliography,
but they are not the only part of it that came from somewhere. What follows is
the same treatment for everything else a frame is made of — the shapes the
shader draws without a slot, the pattern stack, the colour, the light, the
listening and the passes at the end.</p>
<p>A source here is the <em>idea</em>, not the implementation: all of it is
written out in this repository rather than pulled in as a library, so where a
paper and the code disagree the entry says how. Where there is nothing outside
vizz to cite, the line says so <span class="dim">(dashed)</span> rather than
leaving a blank that reads like a missing citation.</p>
</div>

<ul class="jump">
"#);
    for part in ENGINE {
        page.push_str(&format!(
            "  <li><a href=\"#{}\">{}</a></li>\n",
            part.id,
            escape(part.title)
        ));
    }
    page.push_str("</ul>\n");
    for part in ENGINE {
        page.push_str(&format!(
            "\n<h2 id=\"{}\">{}</h2>\n<p class=\"lede\">{}</p>\n",
            part.id, part.title, part.lede,
        ));
        for piece in part.pieces {
            page.push_str("<div class=\"piece\">\n");
            page.push_str(&format!("  <h3>{}</h3>\n", escape(piece.name)));
            page.push_str(&format!("  {}\n", piece.about));
            page.push_str(&format!(
                "  <p class=\"src{}\"><b>Source:</b> {}</p>\n",
                if piece.ours { " ours" } else { "" },
                piece.source
            ));
            page.push_str("</div>\n");
        }
    }
}
