//! Write the catalogue page for the site.
//!
//! ```sh
//! cargo run --example clouds_page -p vizz-mod > site/clouds/index.html
//! ```
//!
//! The page is generated rather than written because it is a list of
//! eighty-one things that already exist as data, and a hand-kept copy
//! of a list like that is wrong within a month. The plates beside each
//! entry come from `vizz-render`'s `plates` example, which draws them
//! with the shipping code; between the two, nothing on the page is a
//! description of vizz written somewhere else.
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
<meta name="description" content="Every point cloud vizz can make from an equation, and every simulation it can run live: what each one is, what it comes from, and who found it.">
<meta property="og:title" content="vizz — clouds from equations">
<meta property="og:description" content="Sixty-six generators and fifteen live simulations, with the paper each one comes from.">
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
    page.push_str(&format!(r#"<h1>Clouds from equations</h1>
<div class="intro">
<p class="lede">vizz draws point clouds. Most of them come from a scanner, a
camera or a file — and {generators} of them come from mathematics, with
{simulations} more that are still running while you watch.</p>

<p>Every picture on this page was drawn by the app's own code, from the same
functions that fill a slot when you pick one from the menu. Nothing here is an
artist's impression of what a generator makes.</p>

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
</div>

<ul class="jump">
"#));
    for group in Group::ALL {
        if CATALOGUE.iter().any(|g| g.group == *group) {
            page.push_str(&format!(
                "  <li><a href=\"#{}\">{}</a></li>\n",
                anchor(*group, "gen"),
                escape(group.label())
            ));
        }
    }
    page.push_str("  <li><a href=\"#alive\">alive</a></li>\n  <li><a href=\"#sources\">sources</a></li>\n</ul>\n");
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
    page.push_str("</ol>\n");
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
