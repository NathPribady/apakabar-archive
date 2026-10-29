//! Small SVG/HTML chart builders. Colours come from CSS variables, so every chart follows the
//! light/dark theme.
//!
//! The time charts are drawn twice: a wide version for screens and a narrow one, with larger
//! type and fewer labels, that the stylesheet shows instead on phones (`.chart .desk` and
//! `.chart .phone`). Bars of one colour are a single path, so a chart of 140 months stays a
//! few hundred elements.

use super::i18n::{self, t};
use super::layout::{esc_attr, fmt_n, pct};
use maud::{html, Markup, PreEscaped};
use std::fmt::Write;

fn x(s: &str) -> String {
    esc_attr(s)
}

/// Rough width of a label in the sans-serif face, in the chart's units.
fn text_w(s: &str, size: f64) -> f64 {
    s.chars().count() as f64 * size * 0.56
}

/// 1,000 as "1k" (English) or "1 rb" (Indonesian), for the narrow charts' axes.
fn compact(v: f64) -> String {
    if v >= 1000.0 {
        let k = v / 1000.0;
        format!("{}{}", i18n::dec(k, if k.fract().abs() < 1e-9 { 0 } else { 1 }), t("k", " rb"))
    } else {
        fmt_n(v as i64)
    }
}

/// Lay legend items out in rows that fit `width`; returns (x, row) per item and the rows.
fn legend_layout(names: &[String], width: f64, size: f64, swatch: f64) -> (Vec<(f64, usize)>, usize) {
    let mut out = Vec::new();
    let (mut xx, mut row) = (0.0, 0);
    for n in names {
        let w = swatch + 5.0 + text_w(n, size) + 14.0;
        if xx > 0.0 && xx + w > width {
            xx = 0.0;
            row += 1;
        }
        out.push((xx, row));
        xx += w;
    }
    (out, if names.is_empty() { 0 } else { row + 1 })
}

/// Width of the wide time charts, in the units of their drawing: about the width of the essay
/// column they sit in, so their type shows at its own size.
const ESSAY_W: f64 = 780.0;

fn wrap(wide: String, narrow: String) -> Markup {
    html! { div.chart { (PreEscaped(wide)) (PreEscaped(narrow)) } }
}

pub struct MonthBar {
    pub ym: String,
    pub parts: Vec<(&'static str, i64)>, // (css var name, count)
}

/// Monthly stacked bars across 1990-2002 with dated event markers and thin-capture ticks.
pub fn monthly_stacked(months: &[MonthBar], labels: &[(&str, &str)], events: &[(String, String)], thin: &[String], link_prefix: Option<&str>) -> Markup {
    wrap(monthly_svg(months, labels, events, thin, link_prefix, false), monthly_svg(months, labels, events, thin, link_prefix, true))
}

fn monthly_svg(months: &[MonthBar], labels: &[(&str, &str)], events: &[(String, String)], thin: &[String], link_prefix: Option<&str>, narrow: bool) -> String {
    let (w, fs) = if narrow { (340.0, 12.0) } else { (ESSAY_W, 13.0) };
    let (l, r, tm) = if narrow { (34.0, 8.0, 26.0) } else { (54.0, 16.0, 34.0) };
    let thin_label = if narrow { t("thin capture", "rekaman tipis") } else { t("thin capture (under a quarter of nearby months)", "rekaman tipis (kurang dari seperempat bulan-bulan di sekitarnya)") };
    let mut names: Vec<String> = labels.iter().map(|(_, n)| n.to_string()).collect();
    if !thin.is_empty() {
        names.push(thin_label.to_string());
    }
    let (lpos, lrows) = legend_layout(&names, w - l - r, fs, 10.0);
    let plot_h = if narrow { 170.0 } else { 240.0 };
    let axis_h = if thin.is_empty() { fs + 10.0 } else { fs + 20.0 };
    let h = tm + plot_h + axis_h + lrows as f64 * (fs + 8.0) + 4.0;
    let base = tm + plot_h;
    let n = months.len().max(1) as f64;
    let bw = (w - l - r) / n;
    let max = months.iter().map(|m| m.parts.iter().map(|p| p.1).sum::<i64>()).max().unwrap_or(1).max(1) as f64;
    let step = nice_step(max, if narrow { 4 } else { 5 });
    let top = (max / step).ceil() * step;
    let y = |v: f64| tm + plot_h * (1.0 - v / top);
    let mut s = String::new();
    write!(s, r#"<svg class="{}" viewBox="0 0 {w} {h:.0}" role="img" aria-label="{}">"#, if narrow { "phone" } else { "desk" }, t("Messages per month", "Pesan per bulan")).unwrap();
    // grid and the count axis
    let mut g = 0.0;
    while g <= top + 0.1 {
        let lab = if narrow { compact(g) } else { fmt_n(g as i64) };
        write!(s, r#"<line x1="{l}" x2="{}" y1="{:.1}" y2="{:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="{fs}" text-anchor="end" fill="var(--ink3)">{lab}</text>"#, w - r, y(g), y(g), l - 5.0, y(g) + fs * 0.35).unwrap();
        g += step;
    }
    // the bars: one path per colour
    let mut paths: Vec<(&str, String)> = Vec::new();
    for (i, m) in months.iter().enumerate() {
        let x0 = l + i as f64 * bw;
        let mut acc = 0.0;
        for (var, c) in &m.parts {
            if *c == 0 {
                continue;
            }
            let (y1, y0) = (y(acc + *c as f64), y(acc));
            let d = match paths.iter_mut().find(|p| p.0 == *var) {
                Some(p) => &mut p.1,
                None => {
                    paths.push((var, String::new()));
                    &mut paths.last_mut().unwrap().1
                }
            };
            write!(d, "M{:.2} {:.2}h{:.2}v{:.2}h{:.2}z", x0 + bw * 0.08, y1, (bw * 0.84).max(0.6), (y0 - y1).max(0.0), -(bw * 0.84).max(0.6)).unwrap();
            acc += *c as f64;
        }
    }
    for (var, d) in &paths {
        write!(s, r#"<path d="{d}" fill="var({var})"/>"#).unwrap();
    }
    // a hover target (and link) per month on the wide chart; links only on the narrow one
    for (i, m) in months.iter().enumerate() {
        let x0 = l + i as f64 * bw;
        let total: i64 = m.parts.iter().map(|p| p.1).sum();
        let link = link_prefix.map(|p| format!("{p}{}", m.ym));
        if narrow && link.is_none() {
            continue;
        }
        if let Some(href) = &link {
            write!(s, r#"<a href="{}">"#, x(href)).unwrap();
        }
        write!(s, r#"<rect x="{x0:.2}" y="{tm}" width="{bw:.2}" height="{plot_h}" fill="transparent">"#).unwrap();
        if !narrow {
            let detail: Vec<String> = m.parts.iter().filter(|p| p.1 > 0).map(|(var, c)| format!("{} {}", labels.iter().find(|lb| lb.0 == *var).map(|lb| lb.1).unwrap_or(""), fmt_n(*c))).collect();
            write!(s, "<title>{}: {} {} ({})</title>", x(&nice_ym(&m.ym)), fmt_n(total), t("messages", "pesan"), x(&detail.join(" · "))).unwrap();
        }
        s.push_str("</rect>");
        if link.is_some() {
            s.push_str("</a>");
        }
    }
    // years under the axis: each January (every other one on the narrow chart), none that
    // would touch the one before
    s.push_str(&format!(r#"<line x1="{l}" x2="{}" y1="{base}" y2="{base}" stroke="var(--ink3)"/>"#, w - r));
    let mut right_of_last = f64::MIN;
    for (i, m) in months.iter().enumerate() {
        let x0 = l + i as f64 * bw;
        let year: i64 = m.ym[..4].parse().unwrap_or(0);
        let jan = m.ym.ends_with("-01");
        let show = if narrow { jan && year % 2 == 0 } else { jan || (i == 0 && m.ym.get(5..7).map(|mo| mo <= "06").unwrap_or(true)) };
        if show {
            let lab = &m.ym[..4];
            let lw = text_w(lab, fs);
            let left = (x0 + 2.0).min(w - 2.0 - lw);
            if left >= right_of_last + 6.0 {
                right_of_last = left + lw;
                write!(s, r#"<line x1="{x0:.1}" x2="{x0:.1}" y1="{base}" y2="{}" stroke="var(--ink3)"/><text x="{left:.1}" y="{:.1}" font-size="{fs}" fill="var(--ink2)">{lab}</text>"#, base + 5.0, base + fs + 5.0).unwrap();
            }
        }
        if thin.iter().any(|t| t == &m.ym) {
            write!(s, r#"<rect x="{:.2}" y="{:.1}" width="{:.2}" height="4" fill="var(--ink3)">"#, x0 + bw * 0.08, base + fs + 10.0, (bw * 0.84).max(0.6)).unwrap();
            if !narrow {
                write!(s, "<title>{}: {}</title>", x(&nice_ym(&m.ym)), t("thin capture", "rekaman tipis")).unwrap();
            }
            s.push_str("</rect>");
        }
    }
    // dated events: numbered markers above the bars
    let rad = if narrow { 8.0 } else { 9.0 };
    for (k, (ym, label)) in events.iter().enumerate() {
        if let Some(i) = months.iter().position(|m| &m.ym == ym) {
            let xm = l + (i as f64 + 0.5) * bw;
            write!(s, r#"<g><title>{}</title><line x1="{xm:.1}" x2="{xm:.1}" y1="{:.1}" y2="{base}" stroke="var(--ink)" stroke-dasharray="2 3" stroke-width="1"/><circle cx="{xm:.1}" cy="{:.1}" r="{rad}" fill="var(--paper)" stroke="var(--ink)"/><text x="{xm:.1}" y="{:.1}" font-size="{}" font-weight="600" text-anchor="middle" fill="var(--ink)">{}</text></g>"#,
                x(label), tm - 6.0 - rad + rad, tm - 6.0 - rad, tm - 6.0 - rad + rad * 0.4, rad * 1.25, k + 1).unwrap();
        }
    }
    // legend
    let ly = base + axis_h + 4.0;
    for (k, name) in names.iter().enumerate() {
        let (lx, row) = lpos[k];
        let yy = ly + row as f64 * (fs + 8.0);
        let thin_item = k == labels.len();
        let fill = if thin_item { "var(--ink3)".to_string() } else { format!("var({})", labels[k].0) };
        let (sh, so) = if thin_item { (4.0, 3.0) } else { (10.0, 0.0) };
        write!(s, r#"<rect x="{:.1}" y="{:.1}" width="10" height="{sh}" rx="2" fill="{fill}"/><text x="{:.1}" y="{:.1}" font-size="{fs}" fill="var(--ink2)">{}</text>"#, l + lx, yy + so, l + lx + 15.0, yy + 9.5, x(name)).unwrap();
    }
    s.push_str("</svg>");
    s
}

pub fn nice_ym(ym: &str) -> String {
    i18n::nice_ym(ym)
}

pub fn nice_step(max: f64, ticks: usize) -> f64 {
    let raw = max / ticks as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nf = if f <= 1.0 { 1.0 } else if f <= 2.0 { 2.0 } else if f <= 2.5 { 2.5 } else if f <= 5.0 { 5.0 } else { 10.0 };
    (nf * mag).max(1e-9)
}

pub struct Series {
    pub name: String,
    pub color: &'static str,
    pub values: Vec<Option<f64>>,
    pub dashed: bool,
}

/// Lines over a shared categorical x axis. `pct_axis` formats the y axis as percentages.
pub fn lines(xlabels: &[String], series: &[Series], pct_axis: bool, markers: &[(usize, String)], label_every: usize, height: f64) -> Markup {
    wrap(lines_svg(xlabels, series, pct_axis, markers, label_every, height, false), lines_svg(xlabels, series, pct_axis, markers, label_every, height, true))
}

fn lines_svg(xlabels: &[String], series: &[Series], pct_axis: bool, markers: &[(usize, String)], label_every: usize, height: f64, narrow: bool) -> String {
    let (w, fs) = if narrow { (340.0, 12.0) } else { (ESSAY_W, 13.0) };
    let (l, r, tp) = if narrow { (36.0, 10.0, if markers.is_empty() { 10.0 } else { 30.0 }) } else { (58.0, 170.0, if markers.is_empty() { 14.0 } else { 26.0 }) };
    let plot_h = if narrow { (height * 0.72).max(150.0) } else { height - 44.0 };
    // on the narrow chart the series are named in a legend below, not at the line ends
    let names: Vec<String> = series.iter().map(|s| s.name.clone()).collect();
    let (lpos, lrows) = if narrow { legend_layout(&names, w - l - r, fs, 18.0) } else { (Vec::new(), 0) };
    let h = tp + plot_h + fs + 12.0 + lrows as f64 * (fs + 8.0) + if narrow { 4.0 } else { 0.0 };
    let n = xlabels.len().max(2) as f64;
    let max = series.iter().flat_map(|s| s.values.iter().flatten()).cloned().fold(0.0, f64::max).max(if pct_axis { 0.05 } else { 1.0 });
    let step = nice_step(max, 4);
    let top = (max / step).ceil() * step;
    let xp = |i: usize| l + (w - l - r) * i as f64 / (n - 1.0);
    let yp = |v: f64| tp + plot_h * (1.0 - v / top);
    let mut s = String::new();
    write!(s, r#"<svg class="{}" viewBox="0 0 {w} {h:.0}" role="img" aria-label="{}">"#, if narrow { "phone" } else { "desk" }, x(&names.join(", "))).unwrap();
    let mut g = 0.0;
    while g <= top + 1e-9 {
        let lab = if pct_axis { pct(g) } else if narrow { compact(g) } else { fmt_n(g as i64) };
        write!(s, r#"<line x1="{l}" x2="{}" y1="{:.1}" y2="{:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="{fs}" text-anchor="end" fill="var(--ink3)">{lab}</text>"#, w - r, yp(g), yp(g), l - 5.0, yp(g) + fs * 0.35).unwrap();
        g += step;
    }
    // x labels. A quarter ("1996-Q1") is labelled by its year, at its first quarter; only as
    // many labels are kept as fit, and none that would touch the one before.
    let quarterly = xlabels.iter().all(|lb| lb.len() == 7 && lb.as_bytes()[4] == b'-' && lb.as_bytes()[5] == b'Q');
    let short = |lab: &str| -> String { if quarterly { lab[..4].to_string() } else { lab.to_string() } };
    let cand: Vec<usize> = if quarterly { (0..xlabels.len()).filter(|&i| xlabels[i].ends_with("Q1")).collect() } else { (0..xlabels.len()).filter(|i| i % label_every.max(1) == 0).collect() };
    let lw = cand.iter().map(|&i| text_w(&short(&xlabels[i]), fs)).fold(0.0, f64::max) + 10.0;
    let mut k = 1;
    while (cand.len() as f64 / k as f64).ceil() * lw > w - l - r {
        k += 1;
    }
    let mut right_of_last = f64::MIN;
    for (j, i) in cand.into_iter().enumerate() {
        if j % k != 0 {
            continue;
        }
        let lab = short(&xlabels[i]);
        let half = text_w(&lab, fs) / 2.0;
        let xx = xp(i).max(half + 2.0).min(w - half - 2.0);
        if xx - half < right_of_last + 6.0 {
            continue;
        }
        right_of_last = xx + half;
        write!(s, r#"<text x="{xx:.1}" y="{:.1}" font-size="{fs}" text-anchor="middle" fill="var(--ink2)">{}</text>"#, tp + plot_h + fs + 4.0, x(&lab)).unwrap();
    }
    for (k, (i, lab)) in markers.iter().enumerate() {
        let mf = if narrow { 10.0 } else { 12.0 };
        let yy = if narrow { 10.0 + (k % 2) as f64 * 12.0 } else { tp - 8.0 };
        let xx = xp(*i);
        let anchor = if xx + 4.0 + text_w(lab, mf) > w - r { format!(r#"x="{:.1}" text-anchor="end""#, xx - 4.0) } else { format!(r#"x="{:.1}""#, xx + 4.0) };
        write!(s, r#"<line x1="{xx:.1}" x2="{xx:.1}" y1="{:.1}" y2="{:.1}" stroke="var(--ink3)" stroke-dasharray="3 3"/><text {anchor} y="{yy:.1}" font-size="{mf}" fill="var(--ink2)">{}</text>"#, if narrow { yy + 3.0 } else { tp }, tp + plot_h, x(lab)).unwrap();
    }
    let mut end_labels: Vec<(f64, String, &str)> = Vec::new();
    for se in series {
        let mut d = String::new();
        let mut pen = false;
        for (i, v) in se.values.iter().enumerate() {
            match v {
                Some(v) => {
                    write!(d, "{}{:.1} {:.1}", if pen { " L" } else { " M" }, xp(i), yp(*v)).unwrap();
                    pen = true;
                }
                None => pen = false,
            }
        }
        write!(s, r#"<path d="{d}" fill="none" stroke="var({})" stroke-width="{}"{}/>"#, se.color, if narrow { 1.8 } else { 2.2 }, if se.dashed { r#" stroke-dasharray="5 4""# } else { "" }).unwrap();
        if !narrow {
            for (i, v) in se.values.iter().enumerate() {
                if let Some(v) = v {
                    write!(s, r#"<circle cx="{:.1}" cy="{:.1}" r="7" fill="transparent"><title>{} · {}: {}</title></circle>"#, xp(i), yp(*v), x(&se.name), x(&xlabels[i]), if pct_axis { pct(*v) } else { fmt_n(*v as i64) }).unwrap();
                }
            }
            if let Some(v) = se.values.iter().rev().find_map(|v| *v) {
                end_labels.push((yp(v), se.name.clone(), se.color));
            }
        }
    }
    // end labels (wide chart), beside the last value of each line: pushed apart downwards, and
    // back up from the foot of the chart when lines end close together near zero
    end_labels.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let gap = fs + 2.0;
    let mut ys: Vec<f64> = end_labels.iter().map(|e| e.0).collect();
    for i in 1..ys.len() {
        ys[i] = ys[i].max(ys[i - 1] + gap);
    }
    let foot = tp + plot_h;
    if let Some(last) = ys.last_mut() {
        if *last > foot {
            *last = foot;
            for i in (0..ys.len() - 1).rev() {
                ys[i] = ys[i].min(ys[i + 1] - gap);
            }
        }
    }
    for ((_, name, color), yy) in end_labels.iter().zip(ys) {
        write!(s, r#"<text x="{}" y="{:.1}" font-size="{fs}" font-weight="600" fill="var({color})">{}</text>"#, w - r + 8.0, yy + 4.0, x(name)).unwrap();
    }
    // legend (narrow chart)
    let ly = tp + plot_h + fs + 14.0;
    for (k, se) in series.iter().enumerate() {
        if let Some((lx, row)) = lpos.get(k) {
            let yy = ly + *row as f64 * (fs + 8.0);
            write!(s, r#"<line x1="{:.1}" x2="{:.1}" y1="{:.1}" y2="{:.1}" stroke="var({})" stroke-width="2.5"{}/><text x="{:.1}" y="{:.1}" font-size="{fs}" fill="var(--ink2)">{}</text>"#,
                l + lx, l + lx + 18.0, yy + 5.0, yy + 5.0, se.color, if se.dashed { r#" stroke-dasharray="4 3""# } else { "" }, l + lx + 23.0, yy + 9.0, x(&se.name)).unwrap();
        }
    }
    s.push_str("</svg>");
    s
}

/// A heat table: rows x columns, cell shade = value / row maximum (or global maximum). Dark
/// text on a light scale keeps every number legible; the row names stay in view while the
/// years scroll sideways on a phone.
pub fn heat_table(cols: &[String], rows: &[(String, Option<String>, Vec<f64>, Vec<String>)], per_row: bool, color: &str) -> Markup {
    let gmax = rows.iter().flat_map(|r| r.2.iter()).cloned().fold(0.0, f64::max).max(1e-9);
    html! {
        div.heat {
            table.t {
                thead { tr { th scope="col" { span.vh { (t("Name", "Nama")) } } @for c in cols { th scope="col" { (c) } } } }
                tbody {
                    @for (label, link, vals, tips) in rows {
                        @let rmax = vals.iter().cloned().fold(0.0, f64::max).max(1e-9);
                        tr {
                            th scope="row" { @if let Some(l) = link { a href=(l) { (label) } } @else { (label) } }
                            @for (i, v) in vals.iter().enumerate() {
                                @let a = (v / if per_row { rmax } else { gmax } * 45.0).round();
                                td style=(format!("background:color-mix(in srgb, var({color}) {a}%, transparent)")) title=(tips.get(i).cloned().unwrap_or_default()) {
                                    (tips.get(i).map(|t| t.split(' ').next().unwrap_or("").to_string()).unwrap_or_default())
                                }
                            }
                        }
                    }
                }
            }
        }
        p.swipe { (t("Swipe sideways for every year.", "Geser ke samping untuk semua tahun.")) }
    }
}

/// Horizontal bars (HTML, so long labels wrap on phones).
pub fn hbars(items: &[(String, Option<String>, i64, Option<String>)], color: &str) -> Markup {
    let max = items.iter().map(|i| i.2).max().unwrap_or(1).max(1) as f64;
    html! {
        div.hbars {
            @for (label, link, n, note) in items {
                div.hb {
                    div { @if let Some(l) = link { a href=(l) { (label) } } @else { (label) } @if let Some(nt) = note { " " span.small { (nt) } } }
                    div.bar { div style=(format!("width:{:.1}%;background:var({color})", *n as f64 / max * 100.0)) {} }
                    div.n { (fmt_n(*n)) }
                }
            }
        }
    }
}

/// Daily counts around an event (day 0 = the event): two paths, before and after.
pub fn event_bars(daily: &[(i64, i64)], from: i64, to: i64) -> Markup {
    let w = 300.0;
    let h = 70.0;
    let n = (to - from + 1) as f64;
    let bw = w / n;
    let max = daily.iter().map(|d| d.1).max().unwrap_or(1).max(1) as f64;
    let (mut before, mut after) = (String::new(), String::new());
    let mut peak = (0i64, 0i64);
    for (d, c) in daily {
        if *d < from || *d > to {
            continue;
        }
        if *c > peak.1 {
            peak = (*d, *c);
        }
        let x0 = (*d - from) as f64 * bw;
        let hh = (*c as f64 / max * h).max(1.0);
        let path = if *d < 0 { &mut before } else { &mut after };
        write!(path, "M{x0:.2} {:.2}h{:.2}v{hh:.2}h{:.2}z", h - hh, (bw - 0.4).max(0.5), -(bw - 0.4).max(0.5)).unwrap();
    }
    let label = tf_label(peak);
    let mut s = String::new();
    write!(s, r#"<svg viewBox="0 0 {w} {}" role="img" aria-label="{}">"#, h + 14.0, x(&label)).unwrap();
    write!(s, r#"<path d="{before}" fill="var(--other)"/><path d="{after}" fill="var(--accent)"/>"#).unwrap();
    let dl = t("d", "h");
    let x0 = (0 - from) as f64 * bw;
    write!(s, r#"<line x1="{x0:.1}" x2="{x0:.1}" y1="0" y2="{h}" stroke="var(--ink)" stroke-width="1"/><line x1="0" x2="{w}" y1="{h}" y2="{h}" stroke="var(--ink3)"/><text x="0" y="{}" font-size="10" fill="var(--ink3)">{from:+}{dl}</text><text x="{x0:.1}" y="{}" font-size="10" text-anchor="middle" fill="var(--ink)">{}</text><text x="{w}" y="{}" font-size="10" text-anchor="end" fill="var(--ink3)">{to:+}{dl}</text>"#, h + 12.0, h + 12.0, t("event", "peristiwa"), h + 12.0).unwrap();
    s.push_str("</svg>");
    html! { (PreEscaped(s)) }
}

fn tf_label(peak: (i64, i64)) -> String {
    i18n::tf(
        || format!("Messages per day around the event; the busiest day, day {:+}, has {}", peak.0, fmt_n(peak.1)),
        || format!("Pesan per hari di sekitar peristiwa; hari tersibuk, hari {:+}, memuat {}", peak.0, fmt_n(peak.1)),
    )
}

/// 100% stacked bars per year.
pub fn stacked100(years: &[(String, Vec<(String, i64)>)], cats: &[(&str, &str, &str)]) -> Markup {
    wrap(stacked_svg(years, cats, false), stacked_svg(years, cats, true))
}

fn stacked_svg(years: &[(String, Vec<(String, i64)>)], cats: &[(&str, &str, &str)], narrow: bool) -> String {
    let (w, fs) = if narrow { (340.0, 12.0) } else { (960.0, 13.0) };
    let (l, r, tp) = if narrow { (36.0, 6.0, 8.0) } else { (46.0, 230.0, 10.0) };
    let plot_h = if narrow { 190.0 } else { 260.0 };
    // legend: at the right on the wide chart, in rows below the narrow one
    let names: Vec<String> = cats.iter().map(|c| c.1.to_string()).collect();
    let (lpos, lrows) = if narrow { legend_layout(&names, w - 8.0, fs, 11.0) } else { (Vec::new(), 0) };
    let h = tp + plot_h + fs + 14.0 + lrows as f64 * (fs + 8.0);
    let n = years.len().max(1) as f64;
    let bw = (w - l - r) / n;
    // on the narrow chart every other year is labelled when the labels would touch
    let every = if narrow && text_w("\u{2019}00", fs) + 4.0 > bw { 2 } else { 1 };
    let mut s = String::new();
    write!(s, r#"<svg class="{}" viewBox="0 0 {w} {h:.0}" role="img" aria-label="{}">"#, if narrow { "phone" } else { "desk" }, t("Outlets by kind, per year", "Media menurut jenis, per tahun")).unwrap();
    for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let yy = tp + plot_h * (1.0 - p);
        write!(s, r#"<line x1="{l}" x2="{}" y1="{yy:.1}" y2="{yy:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="{fs}" text-anchor="end" fill="var(--ink3)">{}</text>"#, w - r, l - 5.0, yy + fs * 0.35, pct(p)).unwrap();
    }
    for (i, (yr, parts)) in years.iter().enumerate() {
        let tot: i64 = parts.iter().map(|p| p.1).sum::<i64>().max(1);
        let mut acc = 0.0;
        let x0 = l + i as f64 * bw;
        for (key, name, var) in cats {
            let c = parts.iter().find(|p| p.0 == *key).map(|p| p.1).unwrap_or(0);
            if c == 0 {
                continue;
            }
            let share = c as f64 / tot as f64;
            let y1 = tp + plot_h * (1.0 - acc - share);
            let hh = plot_h * share;
            write!(s, r#"<rect x="{:.1}" y="{y1:.1}" width="{:.1}" height="{hh:.1}" fill="var({var})">"#, x0 + bw * 0.1, bw * 0.8).unwrap();
            if !narrow {
                write!(s, "<title>{yr}: {} {} ({})</title>", fmt_n(c), x(name), pct(share)).unwrap();
            }
            s.push_str("</rect>");
            acc += share;
        }
        if i % every == 0 {
            let ylab = if narrow { format!("\u{2019}{}", yr.get(2..4).unwrap_or(yr)) } else { yr.clone() };
            write!(s, r#"<text x="{:.1}" y="{:.1}" font-size="{fs}" text-anchor="middle" fill="var(--ink2)">{ylab}</text>"#, x0 + bw / 2.0, tp + plot_h + fs + 4.0).unwrap();
        }
    }
    for (k, (_, name, var)) in cats.iter().enumerate() {
        let (lx, yy) = if narrow {
            let (px, row) = lpos[k];
            (4.0 + px, tp + plot_h + fs + 14.0 + row as f64 * (fs + 8.0))
        } else {
            (w - r + 16.0, tp + 6.0 + k as f64 * (fs + 8.0))
        };
        write!(s, r#"<rect x="{lx:.1}" y="{yy:.1}" width="11" height="11" rx="2" fill="var({var})"/><text x="{:.1}" y="{:.1}" font-size="{fs}" fill="var(--ink2)">{}</text>"#, lx + 16.0, yy + 9.5, x(name)).unwrap();
    }
    s.push_str("</svg>");
    s
}

/// A tiny sparkline (monthly shares) for theme lists.
pub fn sparkline(values: &[f64], color: &str) -> Markup {
    let w = 220.0;
    let h = 34.0;
    let max = values.iter().cloned().fold(0.0, f64::max).max(1e-9);
    let n = values.len().max(2) as f64;
    let mut d = String::new();
    for (i, v) in values.iter().enumerate() {
        write!(d, "{}{:.1} {:.1}", if i == 0 { "M" } else { " L" }, w * i as f64 / (n - 1.0), h - 2.0 - (h - 4.0) * v / max).unwrap();
    }
    html! { (PreEscaped(format!(r#"<svg viewBox="0 0 {w} {h}" style="width:220px;height:34px" aria-hidden="true"><path d="{d}" fill="none" stroke="var({color})" stroke-width="1.5"/></svg>"#))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn months() -> Vec<MonthBar> {
        (0..30).map(|k| MonthBar { ym: format!("{}-{:02}", 1990 + (k + 9) / 12, (k + 9) % 12 + 1), parts: vec![("--id", 10 + k as i64), ("--en", 5)] }).collect()
    }

    #[test]
    fn both_widths_and_few_elements() {
        let m = monthly_stacked(&months(), &[("--id", "Indonesian"), ("--en", "English")], &[("1991-01".into(), "an event".into())], &["1991-02".into()], None).into_string();
        assert_eq!(m.matches("<svg").count(), 2);
        assert!(m.contains(r#"class="desk""#) && m.contains(r#"class="phone""#));
        // one path per colour and chart, not one rectangle per month and colour
        assert_eq!(m.matches("<path").count(), 4);
        // every label escaped
        let e = monthly_stacked(&months(), &[("--id", "<b>")], &[], &[], None).into_string();
        assert!(!e.contains("<b>") && e.contains("&lt;b&gt;"));
    }

    #[test]
    fn narrow_labels_fit() {
        let labels: Vec<String> = (1993..2002).flat_map(|y| (1..=4).map(move |q| format!("{y}-Q{q}"))).collect();
        let series = [Series { name: "Underground".into(), color: "--accent", values: labels.iter().enumerate().map(|(i, _)| Some(i as f64 / 40.0)).collect(), dashed: false }];
        let svg = lines_svg(&labels, &series, true, &[], 4, 300.0, true);
        let shown = svg.matches(r#"text-anchor="middle""#).count();
        assert!(shown <= 9 && shown >= 3, "{shown} labels");
        assert!(!svg.contains("-Q"), "quarters are shown by their year on phones");
    }
}
