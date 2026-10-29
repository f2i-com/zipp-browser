//! Fonts compiled into the binary.
//!
//! The browser used to reach for a hardcoded list of `C:\Windows\Fonts\*.ttf`
//! paths at startup. That made text rendering depend on which fonts a
//! particular Windows install happened to ship, and produced no text at all
//! anywhere else. Nothing here touches the filesystem: the faces are
//! `include_bytes!`'d, so a build renders identically on any machine and on any
//! platform.
//!
//! The Liberation family is used because it is metric-compatible with the three
//! families the web actually asks for by name — Liberation Sans matches Arial
//! and Helvetica, Liberation Serif matches Times New Roman, Liberation Mono
//! matches Courier New. Substituting a metric-compatible face keeps advances and
//! line breaks where the page's author expected them, which an arbitrary
//! fallback would not.
//!
//! Licensed under the SIL Open Font License 1.1; the license and authors files
//! sit next to the fonts in `assets/fonts/`.

use parley::fontique::{Blob, Collection, FallbackKey, FamilyId, GenericFamily, Script};
use std::sync::Arc;

/// Which CSS generic a face should back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    Sans,
    Serif,
    Mono,
}

/// Every embedded face, with the generic family it belongs to.
///
/// All four styles of each family are present. Dropping the italic or bold
/// faces would not lose the style — it would make the rasterizer synthesise one
/// by shearing or smearing the regular face, which looks visibly wrong next to
/// a real italic.
const FACES: &[(Generic, &[u8])] = &[
    (Generic::Sans, include_bytes!("../assets/fonts/LiberationSans-Regular.ttf")),
    (Generic::Sans, include_bytes!("../assets/fonts/LiberationSans-Bold.ttf")),
    (Generic::Sans, include_bytes!("../assets/fonts/LiberationSans-Italic.ttf")),
    (Generic::Sans, include_bytes!("../assets/fonts/LiberationSans-BoldItalic.ttf")),
    (Generic::Serif, include_bytes!("../assets/fonts/LiberationSerif-Regular.ttf")),
    (Generic::Serif, include_bytes!("../assets/fonts/LiberationSerif-Bold.ttf")),
    (Generic::Serif, include_bytes!("../assets/fonts/LiberationSerif-Italic.ttf")),
    (Generic::Serif, include_bytes!("../assets/fonts/LiberationSerif-BoldItalic.ttf")),
    (Generic::Mono, include_bytes!("../assets/fonts/LiberationMono-Regular.ttf")),
    (Generic::Mono, include_bytes!("../assets/fonts/LiberationMono-Bold.ttf")),
    (Generic::Mono, include_bytes!("../assets/fonts/LiberationMono-Italic.ttf")),
    (Generic::Mono, include_bytes!("../assets/fonts/LiberationMono-BoldItalic.ttf")),
];

/// An embedded face's bytes, for drawing text outside the page's layout (a
/// canvas's `fillText`): `family` is a CSS family list, matched to serif,
/// monospace or (otherwise) sans-serif by name.
pub fn embedded_face(family: &str, bold: bool, italic: bool) -> &'static [u8] {
    let generic = family
        .split(',')
        .map(|f| f.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase())
        .find_map(|f| {
            if f.contains("mono") || f.contains("courier") || f.contains("consol") || f.contains("menlo") || f.contains("code") {
                Some(Generic::Mono)
            } else if (f.contains("serif") && !f.contains("sans")) || f.contains("times") || f.contains("georgia") || f.contains("garamond") {
                Some(Generic::Serif)
            } else if f.contains("sans") || f.contains("arial") || f.contains("helvetica") || f.contains("system-ui") {
                Some(Generic::Sans)
            } else {
                None
            }
        })
        .unwrap_or(Generic::Sans);
    // Each family's faces are Regular, Bold, Italic, BoldItalic, in that order.
    let style = usize::from(bold) + 2 * usize::from(italic);
    FACES.iter().filter(|(g, _)| *g == generic).nth(style).map_or(FACES[0].1, |(_, bytes)| bytes)
}

/// Register the embedded faces and wire them into the CSS generic families.
///
/// The generics are APPENDED, not replaced: where the platform has real fonts
/// installed those still win, so a page asking for `sans-serif` on a Windows
/// box keeps getting the same face it always did. The embedded faces sit behind
/// them as the guarantee that `serif`, `sans-serif` and `monospace` always
/// resolve to something — which is what the hardcoded path list was really for.
///
/// Set `ZIPP_EMBEDDED_FONTS_ONLY=1` to put them FIRST instead. Rendering then
/// no longer depends on the host's installed fonts at all, which is what you
/// want when comparing two screenshots taken on different machines.
pub fn register_embedded_fonts(collection: &mut Collection) {
    let embedded_first = std::env::var_os("ZIPP_EMBEDDED_FONTS_ONLY").is_some();

    let mut sans = Vec::new();
    let mut serif = Vec::new();
    let mut mono = Vec::new();

    for (generic, data) in FACES {
        let blob = Blob::new(Arc::new(*data) as _);
        for (family_id, _fonts) in collection.register_fonts(blob, None) {
            let bucket = match generic {
                Generic::Sans => &mut sans,
                Generic::Serif => &mut serif,
                Generic::Mono => &mut mono,
            };
            // All four styles of a family register under ONE family id, so the
            // same id comes back four times.
            if !bucket.contains(&family_id) {
                bucket.push(family_id);
            }
        }
    }

    // `system-ui` and the `ui-*` generics deliberately get the same faces: with
    // no platform UI font to speak of, resolving them to nothing would leave
    // any page that asks for `system-ui` (as this one's CSS does) with no font.
    for (generic, families) in [
        (GenericFamily::SansSerif, &sans),
        (GenericFamily::SystemUi, &sans),
        (GenericFamily::UiSansSerif, &sans),
        (GenericFamily::Serif, &serif),
        (GenericFamily::UiSerif, &serif),
        (GenericFamily::Monospace, &mono),
        (GenericFamily::UiMonospace, &mono),
    ] {
        if embedded_first {
            let existing: Vec<_> = collection.generic_families(generic).collect();
            let ordered: Vec<_> = families.iter().copied().chain(existing).collect();
            collection.set_generic_families(generic, ordered.into_iter());
        } else {
            collection.append_generic_families(generic, families.iter().copied());
        }
    }

    register_symbol_fallbacks(collection);
}

/// Installed fonts that have the symbols text fonts leave out -- ▾ ✓ ✕ ★ ☰ ⌘
/// -- by platform: Windows, macOS, then the usual Linux ones.
const SYMBOL_FAMILIES: &[&str] = &[
    "Segoe UI Symbol",
    "Apple Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "DejaVu Sans",
];

/// The scripts a symbol is looked up under: one in running text takes the
/// script of the text around it, so "Samples ▾" asks Latin's fallback.
const SYMBOL_SCRIPTS: &[&str] = &["Latn", "Zyyy", "Zinh", "Grek", "Cyrl", "Arab", "Hebr"];

/// Put the symbol fonts behind each script's own fallback.
///
/// The platform's fallback for Latin is a text font (Segoe UI on Windows),
/// which has no ▾, so the menu arrow on a "Samples ▾" button was a box where
/// Chrome, falling back per character, finds Segoe UI Symbol. Emoji were
/// unaffected: those fall back to the emoji font on their own.
pub fn register_symbol_fallbacks(collection: &mut Collection) {
    let symbols: Vec<FamilyId> = SYMBOL_FAMILIES
        .iter()
        .filter_map(|name| collection.family_id(name))
        .collect();
    if symbols.is_empty() {
        return;
    }
    for script in SYMBOL_SCRIPTS {
        let key = FallbackKey::new(Script::from_str_unchecked(script), None);
        let mut families: Vec<FamilyId> = collection.fallback_families(key).collect();
        for &id in &symbols {
            if !families.contains(&id) {
                families.push(id);
            }
        }
        collection.set_fallbacks(key, families.into_iter());
    }
}
