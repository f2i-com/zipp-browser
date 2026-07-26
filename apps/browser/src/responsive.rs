//! Responsive layout post-processing for the Zipp browser.
//!
//! The Stylo/Taffy layout engine used by Zipp does not support CSS media queries
//! or responsive breakpoint selectors (e.g. `md:flex`, `lg:text-5xl`). This module
//! bridges that gap by post-processing HTML and CSS so that Tailwind responsive
//! classes are converted into unconditional inline styles, always rendering the
//! "desktop" variant of the page.
//!
//! The pipeline has four stages, run in order by [`apply_desktop_classes`]:
//!
//! 1. **Remove conditional `hidden`** -- If an element has both `hidden` and a
//!    responsive display class like `md:flex`, the `hidden` class is stripped so
//!    the element becomes visible at the desktop breakpoint.
//!
//! 2. **Grid-to-flexbox conversion** -- CSS Grid is not supported by the Taffy layout engine, so
//!    grid containers are rewritten to use `display:flex; flex-wrap:wrap` with
//!    fixed-width children.
//!
//! 3. **Responsive class inlining** -- Tailwind responsive prefixed classes
//!    (`md:*`, `lg:*`) are mapped to their CSS property equivalents and injected
//!    as inline `style` attributes, bypassing the need for escaped-colon selectors.
//!
//! A separate public function, [`strip_media_queries`], removes `@media` wrappers
//! from CSS stylesheets while preserving the rules inside, so all responsive
//! styles apply unconditionally.

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply all desktop-mode responsive transformations to an HTML fragment.
///
/// This is the main entry point called from `main.rs`. It runs the three HTML
/// transformation stages in sequence: hidden-class removal, grid-to-flexbox
/// conversion, and responsive inline style injection.
pub fn apply_desktop_classes(html: &str) -> String {
    let result = remove_hidden_with_responsive_display(html);
    let result = convert_grid_to_flexbox(&result);
    inject_responsive_inline_styles(&result)
}

/// Unwrap `@media` blocks in a CSS string, discarding the media query condition
/// but keeping all CSS rules that were inside the block.
///
/// This makes every responsive rule apply at every viewport width, effectively
/// locking the page to the widest breakpoint.
///
/// Non-media at-rules (e.g. `@keyframes`, `@font-face`) are passed through
/// unchanged.
#[allow(dead_code)]
pub fn strip_media_queries(css: &str) -> String {
    let mut output = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '@' {
            output.push(ch);
            continue;
        }

        // Read the at-rule keyword (e.g. "@media", "@keyframes") into a
        // small stack buffer to avoid heap allocation for every at-rule.
        let mut keyword_buf = [0u8; 64];
        keyword_buf[0] = b'@';
        let mut keyword_len = 1;
        while let Some(&next) = chars.peek() {
            if next.is_alphanumeric() || next == '-' {
                if keyword_len < keyword_buf.len() {
                    keyword_buf[keyword_len] = next as u8;
                    keyword_len += 1;
                }
                chars.next();
            } else {
                break;
            }
        }
        let keyword = std::str::from_utf8(&keyword_buf[..keyword_len]).unwrap_or("@");

        if !keyword.starts_with("@media") {
            // Not a media query -- pass through verbatim
            output.push_str(keyword);
            continue;
        }

        // Capture the media query condition (between @media and {)
        let mut condition = String::new();
        let mut found_brace = false;
        for c in chars.by_ref() {
            if c == '{' { found_brace = true; break; }
            condition.push(c);
        }
        if !found_brace { continue; }

        // Extract inner rules
        let mut depth: u32 = 1;
        let mut inner_rules = String::new();
        for c in chars.by_ref() {
            match c {
                '{' => { depth += 1; inner_rules.push(c); }
                '}' => { depth -= 1; if depth == 0 { break; } inner_rules.push(c); }
                _ => inner_rules.push(c),
            }
        }

        // Decide what to do based on the condition:
        // - min-width: desktop overrides → KEEP (unwrap)
        // - max-width: mobile-only rules → STRIP (discard)
        // - print/screen: keep screen, strip print
        // - no condition or unknown: keep
        let cond_lower = condition.to_lowercase();
        if cond_lower.contains("max-width") || cond_lower.contains("print") {
            // Mobile/print rules - discard to prevent hiding desktop content
            continue;
        }
        // Desktop overrides or generic rules - emit without wrapper
        output.push_str(&inner_rules);
    }

    output
}

// ---------------------------------------------------------------------------
// Internals: class attribute scanning helpers
// ---------------------------------------------------------------------------

/// A located `class="..."` attribute within an HTML string.
struct ClassAttribute {
    /// Byte offset of the `c` in `class="`
    attr_start: usize,
    /// Byte offset of the first character inside the quotes
    value_start: usize,
    /// Byte offset of the closing quote (exclusive)
    value_end: usize,
}

/// Iterate over all `class="..."` attributes in `html`, yielding their positions.
fn find_class_attributes(html: &str) -> Vec<ClassAttribute> {
    let mut results = Vec::new();
    let mut cursor = 0;

    while let Some(offset) = html[cursor..].find("class=\"") {
        let attr_start = cursor + offset;
        let value_start = attr_start + 7; // len("class=\"") == 7

        let Some(quote_offset) = html[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + quote_offset;

        results.push(ClassAttribute { attr_start, value_start, value_end });
        cursor = value_end + 1;
    }

    results
}

/// Return the byte offset of the `<` that opens the tag containing `pos`.
#[inline]
fn find_tag_open(html: &str, pos: usize) -> Option<usize> {
    html[..pos].rfind('<')
}

/// Return the byte offset of the `>` that closes a tag starting at `tag_start`.
#[inline]
fn find_tag_close(html: &str, tag_start: usize) -> Option<usize> {
    html[tag_start..].find('>').map(|off| tag_start + off)
}

/// True if the class list contains any Tailwind responsive breakpoint prefix.
#[inline]
fn has_responsive_prefix(classes: &str) -> bool {
    classes.contains("md:") || classes.contains("lg:")
}

/// True if the class list contains a responsive display class that would make a
/// `hidden` element visible at the desktop breakpoint.
#[inline]
fn has_responsive_display_class(classes: &str) -> bool {
    classes.contains("md:flex")
        || classes.contains("md:block")
        || classes.contains("md:grid")
        || classes.contains("lg:flex")
}

// ---------------------------------------------------------------------------
// Stage 1: Remove "hidden" class when a responsive display override exists
// ---------------------------------------------------------------------------

/// Strip the `hidden` utility class from any element that also carries a
/// responsive display class (e.g. `md:flex`, `lg:flex`).
///
/// Without this, the element would remain `display:none` because the responsive
/// prefix class has no effect in our non-media-query environment.
fn remove_hidden_with_responsive_display(html: &str) -> String {
    let mut result = html.to_string();

    // Process class attributes in reverse order so that earlier byte offsets
    // remain valid as we modify the string.
    let attrs = find_class_attributes(&result);
    for attr in attrs.into_iter().rev() {
        let classes = &result[attr.value_start..attr.value_end];

        if !classes.contains("hidden") || !has_responsive_display_class(classes) {
            continue;
        }

        // Remove "hidden" while preserving surrounding whitespace
        let cleaned: String = classes
            .split_whitespace()
            .filter(|&c| c != "hidden")
            .collect::<Vec<_>>()
            .join(" ");

        result.replace_range(attr.value_start..attr.value_end, &cleaned);
    }

    result
}

// ---------------------------------------------------------------------------
// Stage 2: Convert CSS Grid containers to flexbox
// ---------------------------------------------------------------------------

/// Rewrite `grid` containers to use flexbox, since CSS Grid is unsupported in
/// the Zipp layout engine.
///
/// Grid containers receive `display:flex; flex-wrap:wrap; gap:1.5rem`, and their
/// direct `<div>` children receive a fixed width of 380px to approximate a
/// multi-column card layout.
fn convert_grid_to_flexbox(html: &str) -> String {
    let mut result = html.to_string();

    // Pass 1: Add flexbox inline style to every element with a `grid` class.
    let attrs = find_class_attributes(&result);
    // Process in reverse to keep byte offsets stable.
    for attr in attrs.into_iter().rev() {
        let classes = &result[attr.value_start..attr.value_end];
        let is_grid = classes.split_whitespace().any(|c| c == "grid");
        if !is_grid {
            continue;
        }

        let Some(tag_start) = find_tag_open(&result, attr.attr_start) else { continue };
        let Some(tag_close) = find_tag_close(&result, tag_start) else { continue };

        // Only inject if no style attribute already exists on the tag
        if !result[tag_start..tag_close].contains("style=\"") {
            let style_attr = " style=\"display:flex;flex-wrap:wrap;gap:1.5rem\"";
            result.insert_str(tag_close, style_attr);
        }
    }

    // Pass 2: Find the first flex-wrap container we injected and add fixed
    // widths to its direct child <div> elements.
    add_fixed_widths_to_flex_children(&mut result);

    result
}

/// Locate the first `display:flex;flex-wrap:wrap;gap:1.5rem` container and set
/// `width:380px; flex:0 0 380px` on each of its direct child `<div>` elements.
fn add_fixed_widths_to_flex_children(html: &mut String) {
    const FLEX_MARKER: &str = "display:flex;flex-wrap:wrap;gap:1.5rem";

    let Some(marker_pos) = html.find(FLEX_MARKER) else { return };
    let Some(container_close_offset) = html[marker_pos..].find('>') else { return };
    let content_start = marker_pos + container_close_offset + 1;

    // Walk the DOM fragment to find direct child <div> tags (depth == 1).
    let mut child_tag_ends: Vec<usize> = Vec::new();
    let mut cursor = content_start;
    let mut depth: i32 = 1;

    while cursor < html.len() && depth > 0 {
        if html[cursor..].starts_with("</div") {
            depth -= 1;
            if depth == 0 {
                break;
            }
            cursor += 6; // skip past "</div>"
        } else if html[cursor..].starts_with("<div") {
            if depth == 1 {
                // Record the position of this child's closing `>`
                if let Some(close_offset) = html[cursor..].find('>') {
                    let tag_close = cursor + close_offset;
                    if !html[cursor..tag_close].contains("style=\"") {
                        child_tag_ends.push(tag_close);
                    }
                }
            }
            depth += 1;
            // Advance past the opening tag
            cursor += html[cursor..].find('>').map_or(4, |o| o + 1);
        } else {
            cursor += 1;
        }
    }

    // Insert in reverse order to preserve earlier byte offsets
    for &tag_close in child_tag_ends.iter().rev() {
        let style_attr = " style=\"width:380px;flex:0 0 380px\"";
        html.insert_str(tag_close, style_attr);
    }
}

// ---------------------------------------------------------------------------
// Stage 3: Inject inline styles for Tailwind responsive classes
// ---------------------------------------------------------------------------

/// Convert Tailwind responsive-prefixed utility classes to inline CSS.
///
/// For each element with `md:*` or `lg:*` classes, the corresponding CSS
/// properties are computed and injected as an inline `style` attribute (or
/// appended to an existing one). This sidesteps the need for escaped-colon CSS
/// selectors that may not work in all layout engines.
fn inject_responsive_inline_styles(html: &str) -> String {
    let mut result = html.to_string();

    // We must re-scan after each mutation because byte offsets shift. Process
    // one element at a time from the beginning.
    let mut cursor = 0;

    while let Some(offset) = result[cursor..].find("class=\"") {
        let attr_start = cursor + offset;
        let value_start = attr_start + 7;

        let Some(quote_offset) = result[value_start..].find('"') else { break };
        let value_end = value_start + quote_offset;
        let classes = result[value_start..value_end].to_string();

        if !has_responsive_prefix(&classes) {
            cursor = value_end + 1;
            continue;
        }

        let inline_css = responsive_classes_to_css(&classes);
        if inline_css.is_empty() {
            cursor = value_end + 1;
            continue;
        }

        let Some(tag_start) = find_tag_open(&result, attr_start) else {
            cursor = value_end + 1;
            continue;
        };
        let Some(tag_close) = find_tag_close(&result, tag_start) else {
            cursor = value_end + 1;
            continue;
        };

        let tag_content = &result[tag_start..tag_close];
        if let Some(style_offset) = tag_content.find("style=\"") {
            // Append to existing style attribute
            let inject_pos = tag_start + style_offset + 7; // right after style="
            result.insert_str(inject_pos, &inline_css);
            cursor = inject_pos + inline_css.len() + (value_end - value_start);
        } else {
            // Add a new style attribute just before the closing >
            let attr = format!(" style=\"{}\"", inline_css);
            result.insert_str(tag_close, &attr);
            cursor = tag_close + attr.len();
        }
    }

    result
}

/// Map a set of Tailwind responsive utility classes to their inline CSS
/// equivalents. Returns an empty string if no known mappings are found.
fn responsive_classes_to_css(classes: &str) -> String {
    let mut css = String::new();

    for class in classes.split_whitespace() {
        let property = match class {
            // Display
            "md:flex" | "lg:flex" => "display:flex;",
            "md:block" | "lg:block" => "display:block;",

            // Flex direction
            "md:flex-row" | "lg:flex-row" => "flex-direction:row;",

            // Alignment
            "md:items-start" => "align-items:flex-start;",
            "md:text-left" | "lg:text-left" => "text-align:left;",

            // Width
            "md:w-1/3" => "width:33.333%;",
            "md:w-2/3" => "width:66.666%;",
            "md:w-1/2" => "width:50%;",

            // Margin
            "md:mx-0" | "lg:mx-0" => "margin-left:0;margin-right:0;",
            "md:mb-0" => "margin-bottom:0;",

            // Padding
            "md:pl-12" => "padding-left:3rem;",
            "md:pl-8" => "padding-left:2rem;",
            "md:pr-8" => "padding-right:2rem;",

            // Font size
            "lg:text-5xl" => "font-size:3rem;",
            "lg:text-4xl" => "font-size:2.25rem;",

            // Grid column specs are handled by grid-to-flexbox conversion
            "md:grid-cols-2" | "lg:grid-cols-3" => "",

            _ => "",
        };

        if !property.is_empty() {
            css.push_str(property);
        }
    }

    css
}
