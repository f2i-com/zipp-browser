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

use parley::fontique::{Blob, Collection, GenericFamily};
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
}
