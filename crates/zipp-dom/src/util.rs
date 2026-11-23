use crate::node::{Node, NodeData};
use color::{AlphaColor, Srgb};
use style::color::AbsoluteColor;

pub type Color = AlphaColor<Srgb>;

#[cfg(feature = "svg")]
use std::sync::{Arc, LazyLock};
#[cfg(feature = "svg")]
use usvg::fontdb;
#[cfg(feature = "svg")]
pub(crate) static FONT_DB: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    crate::fonts::load_embedded_into_fontdb(&mut db);
    Arc::new(db)
});

#[derive(Clone, Copy, Debug)]
pub enum ImageType {
    Image,
    Background(usize),
}

/// A point
#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub struct Point<T> {
    /// The x coordinate
    pub x: T,
    /// The y coordinate
    pub y: T,
}

impl Point<f64> {
    pub const ZERO: Self = Point { x: 0.0, y: 0.0 };
}

// Debug print an RcDom
pub fn walk_tree(indent: usize, node: &Node) {
    // Skip all-whitespace text nodes entirely
    if let NodeData::Text(data) = &node.data {
        if data.content.chars().all(|c| c.is_ascii_whitespace()) {
            return;
        }
    }

    print!("{}", " ".repeat(indent));
    let id = node.id;
    match &node.data {
        NodeData::Document => println!("#Document {id}"),

        NodeData::Text(data) => {
            if data.content.chars().all(|c| c.is_ascii_whitespace()) {
                println!("{id} #text: <whitespace>");
            } else {
                let content = data.content.trim();
                if content.len() > 10 {
                    println!(
                        "#text {id}: {}...",
                        content
                            .split_at(content.char_indices().take(10).last().unwrap().0)
                            .0
                            .escape_default()
                    )
                } else {
                    println!("#text {id}: {}", data.content.trim().escape_default())
                }
            }
        }

        NodeData::Comment => println!("<!-- COMMENT {id} -->"),

        NodeData::AnonymousBlock(_) => println!("{id} AnonymousBlock"),

        NodeData::Element(data) => {
            print!("<{} {id}", data.name.local);
            for attr in data.attrs.iter() {
                print!(" {}=\"{}\"", attr.name.local, attr.value);
            }
            if !node.children.is_empty() {
                println!(">");
            } else {
                println!("/>");
            }
        } // NodeData::Doctype {
          //     ref name,
          //     ref public_id,
          //     ref system_id,
          // } => println!("<!DOCTYPE {} \"{}\" \"{}\">", name, public_id, system_id),
          // NodeData::ProcessingInstruction { .. } => unreachable!(),
    }

    if !node.children.is_empty() {
        for child_id in node.children.iter() {
            walk_tree(indent + 2, node.with(*child_id));
        }

        if let NodeData::Element(data) = &node.data {
            println!("{}</{}>", " ".repeat(indent), data.name.local);
        }
    }
}

#[cfg(feature = "svg")]
pub fn parse_svg(source: &[u8]) -> Result<usvg::Tree, usvg::Error> {
    let options = usvg::Options {
        fontdb: Arc::clone(&*FONT_DB),
        font_family: "Liberation Sans".to_string(),
        ..Default::default()
    };

    let tree = usvg::Tree::from_data(source, &options)?;
    Ok(tree)
}

/// Extract CSS rules that declare SVG presentation properties (`fill`, `stroke`,
/// `stop-color`, etc.) so inline `<svg>` subtrees can receive them via an
/// injected `<style>` block for `usvg`.
pub(crate) fn extract_svg_css_rules(css: &str) -> String {
    let bytes = css.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    let mut out = String::new();
    while i < n {
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // Skip /* comments */
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // Read prelude up to `{` or `;`
        let prelude_start = i;
        let mut quote = 0u8;
        let mut paren = 0usize;
        while i < n {
            let b = bytes[i];
            if quote != 0 {
                if b == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if b == quote {
                    quote = 0;
                }
                i += 1;
                continue;
            }
            if i + 1 < n && b == b'/' && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
            if b == b'"' || b == b'\'' {
                quote = b;
                i += 1;
                continue;
            }
            if b == b'(' {
                paren += 1;
            } else if b == b')' {
                paren = paren.saturating_sub(1);
            } else if paren == 0 && (b == b'{' || b == b';') {
                break;
            }
            i += 1;
        }
        if i >= n {
            break;
        }
        if bytes[i] == b';' {
            i += 1;
            continue;
        }
        let prelude = css[prelude_start..i].trim();
        // Find matching `}` for block
        i += 1;
        let block_start = i;
        let mut depth = 1usize;
        quote = 0;
        while i < n && depth > 0 {
            let b = bytes[i];
            if quote != 0 {
                if b == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if b == quote {
                    quote = 0;
                }
                i += 1;
                continue;
            }
            if i + 1 < n && b == b'/' && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
            if b == b'"' || b == b'\'' {
                quote = b;
                i += 1;
                continue;
            }
            if b == b'{' {
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
            }
            i += 1;
        }
        let block_end = if depth == 0 { i - 1 } else { i };
        let inner = &css[block_start..block_end];
        if let Some(at) = prelude.strip_prefix('@') {
            let at_lower = at.trim_start().to_ascii_lowercase();
            if at_lower.starts_with("layer") {
                out.push_str(&extract_svg_css_rules(inner));
            }
            continue;
        }
        // Keep only rules that declare at least one SVG paint property.
        let mut svg_decls = String::new();
        let mut has_paint = false;
        for decl in inner.split(';') {
            let Some((prop, val)) = decl.split_once(':') else {
                continue;
            };
            let prop = prop.trim();
            let prop_lower = prop.to_ascii_lowercase();
            let val = val.trim();
            if val.is_empty() {
                continue;
            }
            let is_paint = matches!(
                prop_lower.as_str(),
                "fill"
                    | "stroke"
                    | "stroke-width"
                    | "stroke-linecap"
                    | "stroke-linejoin"
                    | "stroke-dasharray"
                    | "stroke-dashoffset"
                    | "stroke-miterlimit"
                    | "fill-opacity"
                    | "stroke-opacity"
                    | "fill-rule"
                    | "clip-rule"
                    | "stop-color"
                    | "stop-opacity"
            );
            let is_svg_compat = is_paint
                || matches!(
                    prop_lower.as_str(),
                    "opacity"
                        | "display"
                        | "visibility"
                        | "color"
                        | "font-size"
                        | "font-weight"
                        | "font-family"
                        | "text-anchor"
                        | "dominant-baseline"
                );
            if is_paint {
                has_paint = true;
            }
            if is_svg_compat {
                let clean_val = val.strip_suffix("!important").unwrap_or(val).trim();
                svg_decls.push_str(&prop_lower);
                svg_decls.push(':');
                svg_decls.push_str(clean_val);
                svg_decls.push(';');
            }
        }
        if has_paint && !prelude.is_empty() {
            // Strip CSS comments from prelude and keep simple selectors that
            // `usvg`'s `simplecss` engine understands.
            let mut clean_sels = Vec::new();
            for sel in prelude.split(',') {
                let sel = sel.trim();
                if sel.is_empty() || sel.contains(':') || sel.contains('[') {
                    continue;
                }
                clean_sels.push(sel);
            }
            if !clean_sels.is_empty() {
                out.push_str(&clean_sels.join(","));
                out.push('{');
                out.push_str(&svg_decls);
                out.push('}');
            }
        }
    }
    out
}

pub trait ToColorColor {
    /// Converts a color into the `AlphaColor<Srgb>` type from the `color` crate
    fn as_color_color(&self) -> Color;
}
impl ToColorColor for AbsoluteColor {
    fn as_color_color(&self) -> Color {
        Color::new(
            *self
                .to_color_space(style::color::ColorSpace::Srgb)
                .raw_components(),
        )
    }
}

/// Creates an markup5ever::QualName.
/// Given a local name and an optional namespace
#[macro_export]
macro_rules! qual_name {
    ($local:tt $(, $ns:ident)?) => {
        $crate::QualName {
            prefix: None,
            ns: $crate::ns!($($ns)?),
            local: $crate::local_name!($local),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::extract_svg_css_rules;

    #[test]
    fn extracts_svg_paint_rules_including_layers_and_drops_non_svg() {
        let css = r#"
            body { color: white; margin: 0; }
            .logo-mark path { fill: var(--brand-color) !important; stroke: currentColor; margin: 4px; }
            .logo-mark:hover path { fill: red; }
            @layer components {
                .icon .accent { stroke-width: 2px; fill: none; }
            }
        "#;
        let extracted = extract_svg_css_rules(css);
        assert!(extracted.contains(".logo-mark path{"), "{extracted}");
        assert!(extracted.contains("fill:var(--brand-color);"), "{extracted}");
        assert!(extracted.contains("stroke:currentColor;"), "{extracted}");
        assert!(!extracted.contains("margin"), "{extracted}");
        assert!(!extracted.contains(":hover"), "{extracted}");
        assert!(extracted.contains(".icon .accent{stroke-width:2px;fill:none;}"), "{extracted}");
    }
}
