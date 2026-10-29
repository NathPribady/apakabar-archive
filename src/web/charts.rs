//! Small SVG/HTML chart builders. Colours come from CSS variables, so every chart
//! follows the light/dark theme. Each data mark carries a <title> for hover.

use super::i18n::{self, t};
use super::layout::{esc_attr, fmt_n, pct};
use maud::{html, Markup, PreEscaped};
use std::fmt::Write;

fn x(s: &str) -> String {
    esc_attr(s)
}

pub struct MonthBar {
    pub ym: String,
    pub parts: Vec<(&'static str, i64)>, // (css var name, count)
}

/// Monthly stacked bars across 1990-2002 with dated event markers and thin-capture ticks.
pub fn monthly_stacked(months: &[MonthBar], labels: &[(&str, &str)], events: &[(String, String)], thin: &[String], link_prefix: Option<&str>) -> Markup {
    let w = 960.0;
    let h = 320.0;
    let (l, r, tm, b) = (46.0, 26.0, 34.0, 42.0);
    let n = months.len().max(1) as f64;
    let bw = (w - l - r) / n;
    let max = months.iter().map(|m| m.parts.iter().map(|p| p.1).sum::<i64>()).max().unwrap_or(1).max(1) as f64;
    let step = nice_step(max, 5);
    let top = (max / step).ceil() * step;
    let y = |v: f64| tm + (h - tm - b) * (1.0 - v / top);
    let mut s = String::new();
    write!(s, r#"<svg viewBox="0 0 {w} {h}" role="img" aria-label="{}">"#, t("Messages per month", "Pesan per bulan")).unwrap();
    // grid
    let mut g = 0.0;
    while g <= top + 0.1 {
        write!(s, r#"<line x1="{l}" x2="{}" y1="{:.1}" y2="{:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="11" text-anchor="end" fill="var(--ink3)">{}</text>"#, w - r, y(g), y(g), l - 6.0, y(g) + 4.0, fmt_n(g as i64)).unwrap();
        g += step;
    }
    // bars
    for (i, m) in months.iter().enumerate() {
        let x0 = l + i as f64 * bw;
        let mut acc = 0.0;
        let total: i64 = m.parts.iter().map(|p| p.1).sum();
        let mlabel = nice_ym(&m.ym);
        if let Some(pfx) = link_prefix {
            write!(s, r#"<a href="{}{}">"#, pfx, m.ym).unwrap();
        }
        for (var, c) in &m.parts {
            if *c == 0 { continue; }
            let y1 = y(acc + *c as f64);
            let y0 = y(acc);
            write!(s, r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="var({})"><title>{}: {} {} {} {}</title></rect>"#,
                x0 + 0.5, y1, (bw - 1.0).max(0.6), (y0 - y1).max(0.0), var, mlabel, fmt_n(*c), t("of", "dari"), fmt_n(total), t("messages", "pesan")).unwrap();
            acc += *c as f64;
        }
        if link_prefix.is_some() { s.push_str("</a>"); }
        // a year label at each January, and at the first bar unless the next January is close
        if m.ym.ends_with("-01") || (i == 0 && m.ym.get(5..7).map(|mo| mo <= "06").unwrap_or(true)) {
            write!(s, r#"<line x1="{:.1}" x2="{:.1}" y1="{}" y2="{}" stroke="var(--ink3)"/><text x="{:.1}" y="{}" font-size="11" fill="var(--ink2)">{}</text>"#,
                x0, x0, h - b, h - b + 5.0, x0 + 2.0, h - b + 17.0, &m.ym[..4]).unwrap();
        }
        if thin.iter().any(|t| t == &m.ym) {
            write!(s, r#"<rect x="{:.2}" y="{}" width="{:.2}" height="4" fill="var(--ink3)"><title>{}: {}</title></rect>"#, x0 + 0.5, h - b + 24.0, (bw - 1.0).max(0.6), mlabel, t("thin capture", "rekaman tipis")).unwrap();
        }
    }
    write!(s, r#"<line x1="{l}" x2="{}" y1="{}" y2="{}" stroke="var(--ink3)"/>"#, w - r, h - b, h - b).unwrap();
    // events
    for (k, (ym, label)) in events.iter().enumerate() {
        if let Some(i) = months.iter().position(|m| &m.ym == ym) {
            let xm = l + (i as f64 + 0.5) * bw;
            write!(s, r#"<line x1="{xm:.1}" x2="{xm:.1}" y1="{}" y2="{}" stroke="var(--ink)" stroke-dasharray="2 3" stroke-width="1"/><circle cx="{xm:.1}" cy="{}" r="8" fill="var(--paper)" stroke="var(--ink)"/><text x="{xm:.1}" y="{}" font-size="10" font-weight="600" text-anchor="middle" fill="var(--ink)">{}</text><title>{}</title>"#,
                tm - 6.0, h - b, tm - 14.0, tm - 10.5, k + 1, x(label)).unwrap();
        }
    }
    // legend
    let mut lx = l;
    for (var, name) in labels {
        write!(s, r#"<rect x="{lx}" y="{}" width="10" height="10" rx="2" fill="var({var})"/><text x="{}" y="{}" font-size="11.5" fill="var(--ink2)">{}</text>"#, h - 11.0, lx + 14.0, h - 2.0, x(name)).unwrap();
        lx += 22.0 + name.len() as f64 * 6.6;
    }
    if !thin.is_empty() {
        write!(s, r#"<rect x="{lx}" y="{}" width="10" height="4" fill="var(--ink3)"/><text x="{}" y="{}" font-size="11.5" fill="var(--ink2)">{}</text>"#, h - 8.0, lx + 14.0, h - 2.0, t("thin capture (under a quarter of nearby months)", "rekaman tipis (kurang dari seperempat bulan-bulan di sekitarnya)")).unwrap();
    }
    s.push_str("</svg>");
    html! { (PreEscaped(s)) }
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
    let w = 960.0;
    let h = height;
    let (l, r, t, b) = (50.0, 150.0, 18.0, 30.0);
    let n = xlabels.len().max(2) as f64;
    let max = series.iter().flat_map(|s| s.values.iter().flatten()).cloned().fold(0.0, f64::max).max(if pct_axis { 0.05 } else { 1.0 });
    let step = nice_step(max, 4);
    let top = (max / step).ceil() * step;
    let xp = |i: usize| l + (w - l - r) * i as f64 / (n - 1.0);
    let yp = |v: f64| t + (h - t - b) * (1.0 - v / top);
    let mut s = String::new();
    write!(s, r#"<svg viewBox="0 0 {w} {h}" role="img">"#).unwrap();
    let mut g = 0.0;
    while g <= top + 1e-9 {
        let lab = if pct_axis { pct(g) } else { fmt_n(g as i64) };
        write!(s, r#"<line x1="{l}" x2="{}" y1="{:.1}" y2="{:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="11" text-anchor="end" fill="var(--ink3)">{lab}</text>"#, w - r, yp(g), yp(g), l - 6.0, yp(g) + 4.0).unwrap();
        g += step;
    }
    for (i, lab) in xlabels.iter().enumerate() {
        if i % label_every.max(1) == 0 {
            write!(s, r#"<text x="{:.1}" y="{}" font-size="11" text-anchor="middle" fill="var(--ink2)">{}</text>"#, xp(i), h - b + 16.0, x(lab)).unwrap();
        }
    }
    for (i, lab) in markers {
        write!(s, r#"<line x1="{:.1}" x2="{:.1}" y1="{t}" y2="{}" stroke="var(--ink3)" stroke-dasharray="3 3"/><text x="{:.1}" y="{}" font-size="10.5" fill="var(--ink2)">{}</text>"#, xp(*i), xp(*i), h - b, xp(*i) + 4.0, t + 10.0, x(lab)).unwrap();
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
        write!(s, r#"<path d="{d}" fill="none" stroke="var({})" stroke-width="2"{}/>"#, se.color, if se.dashed { r#" stroke-dasharray="5 4""# } else { "" }).unwrap();
        for (i, v) in se.values.iter().enumerate() {
            if let Some(v) = v {
                write!(s, r#"<circle cx="{:.1}" cy="{:.1}" r="6" fill="transparent"><title>{} · {}: {}</title></circle>"#, xp(i), yp(*v), x(&se.name), x(&xlabels[i]), if pct_axis { pct(*v) } else { fmt_n(*v as i64) }).unwrap();
            }
        }
        if let Some((i, v)) = se.values.iter().enumerate().rev().find_map(|(i, v)| v.map(|v| (i, v))) {
            end_labels.push((yp(v), se.name.clone(), se.color));
            let _ = i;
        }
    }
    // end labels, nudged apart
    end_labels.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut last = -100.0;
    for (yv, name, color) in end_labels {
        let yy = if yv - last < 14.0 { last + 14.0 } else { yv };
        last = yy;
        write!(s, r#"<text x="{}" y="{:.1}" font-size="12" font-weight="600" fill="var({color})">{}</text>"#, w - r + 8.0, yy + 4.0, x(&name)).unwrap();
    }
    s.push_str("</svg>");
    html! { (PreEscaped(s)) }
}

/// A heat table: rows x columns, cell shade = value / row maximum (or global maximum).
pub fn heat_table(cols: &[String], rows: &[(String, Option<String>, Vec<f64>, Vec<String>)], per_row: bool, color: &str) -> Markup {
    let gmax = rows.iter().flat_map(|r| r.2.iter()).cloned().fold(0.0, f64::max).max(1e-9);
    html! {
        div style="overflow-x:auto" {
            table.t style="font-size:12.5px" {
                thead { tr { th {} @for c in cols { th style="text-align:center;font-weight:500" { (c) } } } }
                tbody {
                    @for (label, link, vals, tips) in rows {
                        @let rmax = vals.iter().cloned().fold(0.0, f64::max).max(1e-9);
                        tr {
                            td style="white-space:nowrap" { @if let Some(l) = link { a href=(l) { (label) } } @else { (label) } }
                            @for (i, v) in vals.iter().enumerate() {
                                @let a = (v / if per_row { rmax } else { gmax } * 100.0).round();
                                td style=(format!("text-align:center;padding:5px 2px;background:color-mix(in srgb, var({color}) {a}%, transparent);color:{}", if a > 55.0 { "var(--paper)" } else { "var(--ink2)" })) title=(tips.get(i).cloned().unwrap_or_default()) {
                                    (tips.get(i).map(|t| t.split(' ').next().unwrap_or("").to_string()).unwrap_or_default())
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Horizontal bars (HTML, so long labels wrap on phones).
pub fn hbars(items: &[(String, Option<String>, i64, Option<String>)], color: &str) -> Markup {
    let max = items.iter().map(|i| i.2).max().unwrap_or(1).max(1) as f64;
    html! {
        div style="font:13.5px/1.35 var(--sans)" {
            @for (label, link, n, note) in items {
                div style="display:grid;grid-template-columns:minmax(120px,38%) 1fr 64px;gap:10px;align-items:center;margin:4px 0" {
                    div { @if let Some(l) = link { a href=(l) { (label) } } @else { (label) } @if let Some(nt) = note { " " span.small { (nt) } } }
                    div style="background:var(--chip);border-radius:3px;height:14px" { div style=(format!("width:{:.1}%;height:14px;border-radius:3px;background:var({color})", *n as f64 / max * 100.0)) {} }
                    div style="text-align:right;font-variant-numeric:tabular-nums;color:var(--ink2)" { (fmt_n(*n)) }
                }
            }
        }
    }
}

/// Daily counts around an event (day 0 = the event).
pub fn event_bars(daily: &[(i64, i64)], from: i64, to: i64) -> Markup {
    let w = 300.0;
    let h = 70.0;
    let n = (to - from + 1) as f64;
    let bw = w / n;
    let max = daily.iter().map(|d| d.1).max().unwrap_or(1).max(1) as f64;
    let mut s = String::new();
    write!(s, r#"<svg viewBox="0 0 {w} {}" role="img" aria-label="{}">"#, h + 14.0, t("Messages per day around the event", "Pesan per hari di sekitar peristiwa")).unwrap();
    let dl = t("d", "h");
    let day = t("day", "hari");
    for (d, c) in daily {
        if *d < from || *d > to { continue; }
        let x0 = (*d - from) as f64 * bw;
        let hh = (*c as f64 / max * h).max(1.0);
        let col = if *d < 0 { "var(--other)" } else { "var(--accent)" };
        write!(s, r#"<rect x="{x0:.2}" y="{:.2}" width="{:.2}" height="{hh:.2}" fill="{col}"><title>{day} {d:+}: {c}</title></rect>"#, h - hh, (bw - 0.4).max(0.5)).unwrap();
    }
    let x0 = (0 - from) as f64 * bw;
    write!(s, r#"<line x1="{x0:.1}" x2="{x0:.1}" y1="0" y2="{h}" stroke="var(--ink)" stroke-width="1"/><line x1="0" x2="{w}" y1="{h}" y2="{h}" stroke="var(--ink3)"/><text x="0" y="{}" font-size="10" fill="var(--ink3)">{from:+}{dl}</text><text x="{x0:.1}" y="{}" font-size="10" text-anchor="middle" fill="var(--ink)">{}</text><text x="{w}" y="{}" font-size="10" text-anchor="end" fill="var(--ink3)">{to:+}{dl}</text>"#, h + 12.0, h + 12.0, t("event", "peristiwa"), h + 12.0).unwrap();
    s.push_str("</svg>");
    html! { (PreEscaped(s)) }
}

/// 100% stacked bars per year.
pub fn stacked100(years: &[(String, Vec<(String, i64)>)], cats: &[(&str, &str, &str)]) -> Markup {
    let w = 960.0;
    let h = 300.0;
    let (l, r, t, b) = (40.0, 210.0, 10.0, 26.0);
    let n = years.len().max(1) as f64;
    let bw = (w - l - r) / n;
    let mut s = String::new();
    write!(s, r#"<svg viewBox="0 0 {w} {h}" role="img">"#).unwrap();
    for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let yy = t + (h - t - b) * (1.0 - p);
        write!(s, r#"<line x1="{l}" x2="{}" y1="{yy:.1}" y2="{yy:.1}" stroke="var(--rule)"/><text x="{}" y="{:.1}" font-size="11" text-anchor="end" fill="var(--ink3)">{}</text>"#, w - r, l - 6.0, yy + 4.0, pct(p)).unwrap();
    }
    for (i, (yr, parts)) in years.iter().enumerate() {
        let tot: i64 = parts.iter().map(|p| p.1).sum::<i64>().max(1);
        let mut acc = 0.0;
        let x0 = l + i as f64 * bw;
        for (key, name, var) in cats {
            let c = parts.iter().find(|p| p.0 == *key).map(|p| p.1).unwrap_or(0);
            if c == 0 { continue; }
            let share = c as f64 / tot as f64;
            let y1 = t + (h - t - b) * (1.0 - acc - share);
            let hh = (h - t - b) * share;
            write!(s, r#"<rect x="{:.1}" y="{y1:.1}" width="{:.1}" height="{hh:.1}" fill="var({var})"><title>{yr}: {} {} ({})</title></rect>"#, x0 + 2.0, bw - 4.0, fmt_n(c), x(name), pct(share)).unwrap();
            acc += share;
        }
        write!(s, r#"<text x="{:.1}" y="{}" font-size="11" text-anchor="middle" fill="var(--ink2)">{}</text>"#, x0 + bw / 2.0, h - 8.0, yr).unwrap();
    }
    for (k, (_, name, var)) in cats.iter().enumerate() {
        let yy = t + 6.0 + k as f64 * 19.0;
        write!(s, r#"<rect x="{}" y="{yy}" width="11" height="11" rx="2" fill="var({var})"/><text x="{}" y="{}" font-size="12" fill="var(--ink2)">{}</text>"#, w - r + 14.0, w - r + 30.0, yy + 10.0, x(name)).unwrap();
    }
    s.push_str("</svg>");
    html! { (PreEscaped(s)) }
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
