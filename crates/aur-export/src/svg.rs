//! Formula preview: one TeX formula rendered to inline SVG by Typst.
//!
//! The preview sheet and the PDF must agree on how a formula looks, so the
//! preview does not use a second math engine — it runs the very same Typst
//! compiler on a one-formula document and turns the page into SVG.

use std::collections::HashMap;
use std::path::PathBuf;

use typst_layout::PagedDocument;

use crate::fonts;
use crate::math;
use crate::world::TypstWorld;

/// A rendered formula, ready to be dropped into the HTML preview.
#[derive(Debug, Clone)]
pub struct MathSvg {
    /// Self-contained inline `<svg>`; glyphs are outlined, no font needed.
    pub svg: String,
    pub width_em: f64,
    pub height_em: f64,
}

/// Render `tex` (TeX source, without the surrounding dollars).
///
/// `size_pt` is the preview font size; heights are reported in `em` so the
/// caller can scale the formula with the surrounding text.
pub fn render_math(
    tex: &str,
    display: bool,
    size_pt: f64,
    font_paths: &[PathBuf],
) -> Result<MathSvg, String> {
    let tex = tex.trim();
    if tex.is_empty() {
        return Err("空公式".into());
    }
    if tex.len() > 4000 {
        return Err("公式过长".into());
    }

    let size = if size_pt.is_finite() {
        size_pt.clamp(6.0, 48.0)
    } else {
        16.0
    };

    let body = math::convert(tex);
    // Typst distinguishes inline `$x$` from block `$ x $` only by padding.
    let expr = if display {
        format!("$ {body} $")
    } else {
        format!("${body}$")
    };

    // `width: auto, height: auto` makes the page hug the formula exactly, so
    // the SVG viewBox is the formula box — no whitespace to crop by hand.
    let source = format!(
        "#set page(width: auto, height: auto, margin: 1.5pt, fill: none)\n\
         #set text(size: {size}pt, lang: \"zh\", region: \"cn\")\n\
         #set par(justify: false, leading: 0.55em)\n\
         #show math.equation: set text(fill: black)\n\
         {expr}\n"
    );

    let world = TypstWorld::new(source, fonts::load(font_paths), HashMap::new());
    let warned = typst::compile::<PagedDocument>(&world);
    let document = warned.output.map_err(|errors| {
        let mut msg = String::new();
        for (i, err) in errors.iter().take(3).enumerate() {
            if i > 0 {
                msg.push('；');
            }
            msg.push_str(&err.message);
        }
        msg
    })?;

    let page = document.pages().first().ok_or("公式未产生页面")?;
    let raw = typst_svg::svg(page, &typst_svg::SvgOptions::default());

    inline_svg(&raw, display, size)
}

/// Re-shape Typst's page SVG into an inline, em-scaled `<svg>` element.
///
/// Typst emits `width="…pt" height="…pt"`, which would freeze the formula at
/// print size inside a browser at 100% zoom.  We drop both and drive the size
/// from the viewBox so the formula follows the preview font size.
fn inline_svg(raw: &str, display: bool, size_pt: f64) -> Result<MathSvg, String> {
    let start = raw.find("<svg").ok_or("缺少 svg 根元素")?;
    let head_len = raw[start..].find('>').ok_or("svg 根标签未闭合")?;
    let head_end = start + head_len; // index of '>'
    let attrs = &raw[start + 4..head_end];
    let body = &raw[head_end + 1..];

    let (kept, view_box) = filter_attrs(attrs);
    let dims = parse_view_box(&view_box).ok_or("svg 缺少有效 viewBox")?;
    let (w_pt, h_pt) = dims;

    // Typst laid the formula out at `size_pt`, so points convert to em by
    // dividing by that size: a 10pt glyph at 10pt text is exactly 1em.
    let width_em = (w_pt / size_pt).clamp(0.01, 400.0);
    let height_em = (h_pt / size_pt).clamp(0.01, 60.0);

    let (class, style) = if display {
        (
            "math-root block",
            format!("height:{height_em:.3}em;max-width:100%;display:block;margin:0.45em auto"),
        )
    } else {
        (
            "math-root inline",
            format!("height:{height_em:.3}em;max-width:100%;vertical-align:-0.28em"),
        )
    };

    let mut out = String::with_capacity(raw.len() + 160);
    out.push_str("<svg");
    out.push_str(&kept);
    out.push_str(&format!(
        " viewBox=\"{view_box}\" class=\"{class}\" style=\"{style}\" preserveAspectRatio=\"xMinYMin meet\">"
    ));
    out.push_str(body);

    Ok(MathSvg {
        svg: out,
        width_em,
        height_em,
    })
}

/// Copy attributes we want to keep, dropping `width`, `height`, `class`,
/// `style` and `viewBox` (all of which we set ourselves).
fn filter_attrs(attrs: &str) -> (String, String) {
    let mut kept = String::new();
    let mut view_box = String::new();
    let bytes: Vec<char> = attrs.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Attribute name.
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_whitespace() && bytes[i] != '=' {
            i += 1;
        }
        let name: String = bytes[name_start..i].iter().collect();
        if name.is_empty() {
            i += 1;
            continue;
        }
        // Optional value.
        let mut value: Option<String> = None;
        if i < bytes.len() && bytes[i] == '=' {
            i += 1;
            if i < bytes.len() && (bytes[i] == '"' || bytes[i] == '\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = Some(bytes[v_start..i].iter().collect());
                i += 1;
            } else {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_whitespace() {
                    i += 1;
                }
                value = Some(bytes[v_start..i].iter().collect());
            }
        }

        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "viewbox" => {
                if let Some(v) = value {
                    view_box = v;
                }
            }
            "width" | "height" | "class" | "style" => {}
            _ => {
                kept.push(' ');
                kept.push_str(&name);
                if let Some(v) = value {
                    kept.push_str("=\"");
                    kept.push_str(&v.replace('"', "&quot;"));
                    kept.push('"');
                }
            }
        }
    }

    if view_box.is_empty() {
        // Extremely unlikely, but a page SVG always has one.
        view_box = "0 0 1 1".to_string();
    }
    (kept, view_box)
}

fn parse_view_box(vb: &str) -> Option<(f64, f64)> {
    let nums: Vec<f64> = vb
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    if nums.len() != 4 || nums[2] <= 0.0 || nums[3] <= 0.0 {
        return None;
    }
    Some((nums[2], nums[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_view_box() {
        assert_eq!(parse_view_box("0 0 12.5 8"), Some((12.5, 8.0)));
        assert_eq!(parse_view_box("0,0,3,4"), Some((3.0, 4.0)));
        assert_eq!(parse_view_box("0 0 0 8"), None);
        assert_eq!(parse_view_box("garbage"), None);
    }

    #[test]
    fn rewrites_root_and_drops_pt_sizes() {
        let raw = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20pt\" height=\"10pt\" viewBox=\"0 0 20 10\"><path d=\"M0 0\"/></svg>";
        let out = inline_svg(raw, false, 10.0).expect("svg");
        assert!(out.svg.contains("viewBox=\"0 0 20 10\""));
        assert!(!out.svg.contains("width=\"20pt\""));
        assert!(!out.svg.contains("height=\"10pt\""));
        assert!(out.svg.contains("height:1.000em"));
        assert!(out.svg.contains("<path d=\"M0 0\"/>"));
        assert_eq!(out.width_em, 2.0);
    }

    #[test]
    fn keeps_other_attributes() {
        let raw = "<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"1pt\" height=\"1pt\" viewBox=\"0 0 1 1\" data-x=\"y\"></svg>";
        let out = inline_svg(raw, true, 16.0).expect("svg");
        assert!(out.svg.contains("xmlns:xlink"));
        assert!(out.svg.contains("data-x=\"y\""));
        assert!(out.svg.contains("math-root block"));
    }
}
