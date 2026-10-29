/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Servo's media feature list and evaluator.
//!
//! (zipp-browser patch: upstream 0.14 had width, scan, resolution, the device
//! pixel ratio and prefers-color-scheme only, so any other feature in a query
//! -- `height`, `orientation`, `hover`, `pointer`, `prefers-reduced-motion`...
//! -- made it fail to parse and its rules were dropped. The rest of Media
//! Queries 4 and 5 are here, answered as a desktop browser with a mouse
//! answers them; see ZIPP_PATCHES.md.)

use crate::derives::*;
use crate::queries::feature::{AllowsRanges, Evaluator, FeatureFlags, QueryFeatureDescription};
use crate::queries::values::{Orientation, PrefersColorScheme};
use crate::values::computed::{CSSPixelLength, Context, Ratio, Resolution};
use crate::Atom;
use std::fmt::Debug;
use std::sync::LazyLock;

/// https://drafts.csswg.org/mediaqueries-4/#width
fn eval_width(context: &Context) -> CSSPixelLength {
    CSSPixelLength::new(context.device().au_viewport_size().width.to_f32_px())
}

/// https://drafts.csswg.org/mediaqueries-4/#height
fn eval_height(context: &Context) -> CSSPixelLength {
    CSSPixelLength::new(context.device().au_viewport_size().height.to_f32_px())
}

/// https://drafts.csswg.org/mediaqueries-4/#aspect-ratio (and, with no screen
/// of its own to report, the deprecated device-aspect-ratio)
fn eval_aspect_ratio(context: &Context) -> Ratio {
    let size = context.device().au_viewport_size();
    Ratio::new(size.width.0 as f32, size.height.0 as f32)
}

/// https://drafts.csswg.org/mediaqueries-4/#orientation
fn eval_orientation(context: &Context, value: Option<Orientation>) -> bool {
    Orientation::eval(context.device().au_viewport_size(), value)
}

#[derive(Clone, Copy, Debug, FromPrimitive, Parse, ToCss)]
#[repr(u8)]
enum Scan {
    Progressive,
    Interlace,
}

/// https://drafts.csswg.org/mediaqueries-4/#scan
fn eval_scan(_: &Context, _: Option<Scan>) -> bool {
    // Since we doesn't support the 'tv' media type, the 'scan' feature never
    // matches.
    false
}

/// https://drafts.csswg.org/mediaqueries-4/#resolution
fn eval_resolution(context: &Context) -> Resolution {
    Resolution::from_dppx(context.device().device_pixel_ratio().0)
}

/// https://compat.spec.whatwg.org/#css-media-queries-webkit-device-pixel-ratio
fn eval_device_pixel_ratio(context: &Context) -> f32 {
    eval_resolution(context).dppx()
}

fn eval_prefers_color_scheme(context: &Context, query_value: Option<PrefersColorScheme>) -> bool {
    match query_value {
        Some(v) => context.device().color_scheme() == v,
        None => true,
    }
}

/// https://drafts.csswg.org/mediaqueries-4/#color: eight bits a component.
fn eval_color(_: &Context) -> i32 {
    8
}

/// https://drafts.csswg.org/mediaqueries-4/#color-index,
/// https://drafts.csswg.org/mediaqueries-4/#monochrome,
/// https://drafts.csswg.org/mediaqueries-4/#grid: none of those.
fn eval_zero(_: &Context) -> i32 {
    0
}

#[derive(Clone, Copy, Debug, FromPrimitive, Parse, ToCss)]
#[repr(u8)]
enum Hover {
    None,
    Hover,
}

/// https://drafts.csswg.org/mediaqueries-4/#hover, any-hover: a mouse.
fn eval_hover(_: &Context, query_value: Option<Hover>) -> bool {
    !matches!(query_value, Some(Hover::None))
}

#[derive(Clone, Copy, Debug, FromPrimitive, Parse, ToCss)]
#[repr(u8)]
enum Pointer {
    None,
    Coarse,
    Fine,
}

/// https://drafts.csswg.org/mediaqueries-4/#pointer, any-pointer: a mouse.
fn eval_pointer(_: &Context, query_value: Option<Pointer>) -> bool {
    matches!(query_value, None | Some(Pointer::Fine))
}

/// A feature whose value is a keyword, and that matches its first keyword --
/// the one that also matches the boolean form, or its absence -- and no other.
macro_rules! first_keyword_feature {
    ($name:ident, $eval:ident { $first:ident $(, $rest:ident)* $(,)? }, $boolean:expr) => {
        #[derive(Clone, Copy, Debug, FromPrimitive, Parse, PartialEq, ToCss)]
        #[repr(u8)]
        enum $name {
            $first,
            $($rest,)*
        }
        fn $eval(_: &Context, query_value: Option<$name>) -> bool {
            match query_value {
                Some(v) => v == $name::$first,
                None => $boolean,
            }
        }
    };
}

// https://drafts.csswg.org/mediaqueries-5/#prefers-reduced-motion
first_keyword_feature!(PrefersReducedMotion, eval_prefers_reduced_motion { NoPreference, Reduce }, false);
// https://drafts.csswg.org/mediaqueries-5/#prefers-reduced-transparency
first_keyword_feature!(PrefersReducedTransparency, eval_prefers_reduced_transparency { NoPreference, Reduce }, false);
// https://drafts.csswg.org/mediaqueries-5/#prefers-reduced-data
first_keyword_feature!(PrefersReducedData, eval_prefers_reduced_data { NoPreference, Reduce }, false);
// https://drafts.csswg.org/mediaqueries-5/#prefers-contrast
first_keyword_feature!(PrefersContrast, eval_prefers_contrast { NoPreference, More, Less, Custom }, false);
// https://drafts.csswg.org/mediaqueries-5/#forced-colors
first_keyword_feature!(ForcedColors, eval_forced_colors { None, Active }, false);
// https://drafts.csswg.org/mediaqueries-5/#inverted
first_keyword_feature!(InvertedColors, eval_inverted_colors { None, Inverted }, false);
// https://drafts.csswg.org/mediaqueries-5/#dynamic-range (and video-dynamic-range)
first_keyword_feature!(DynamicRange, eval_dynamic_range { Standard, High }, true);
// https://drafts.csswg.org/mediaqueries-4/#color-gamut: sRGB only.
first_keyword_feature!(ColorGamut, eval_color_gamut { Srgb, P3, Rec2020 }, true);
// https://drafts.csswg.org/mediaqueries-5/#scripting
first_keyword_feature!(Scripting, eval_scripting { Enabled, InitialOnly, None }, true);
// https://drafts.csswg.org/mediaqueries-4/#update
first_keyword_feature!(Update, eval_update { Fast, Slow, None }, true);
// https://drafts.csswg.org/mediaqueries-4/#overflow-block
first_keyword_feature!(OverflowBlock, eval_overflow_block { Scroll, None, Paged }, true);
// https://drafts.csswg.org/mediaqueries-4/#overflow-inline
first_keyword_feature!(OverflowInline, eval_overflow_inline { Scroll, None }, true);
// https://w3c.github.io/manifest/#the-display-mode-media-feature
first_keyword_feature!(DisplayMode, eval_display_mode { Browser, MinimalUi, Standalone, Fullscreen, PictureInPicture }, true);

/// A list with all the media features that Servo supports. Built on first use:
/// most of the names are not among Servo's static atoms.
pub static MEDIA_FEATURES: LazyLock<Vec<QueryFeatureDescription>> = LazyLock::new(|| {
    vec![
        feature!(Atom::from("width"), AllowsRanges::Yes, Evaluator::Length(eval_width), FeatureFlags::VIEWPORT_DEPENDENT,),
        feature!(Atom::from("height"), AllowsRanges::Yes, Evaluator::Length(eval_height), FeatureFlags::VIEWPORT_DEPENDENT,),
        feature!(
            Atom::from("aspect-ratio"),
            AllowsRanges::Yes,
            Evaluator::NumberRatio(eval_aspect_ratio),
            FeatureFlags::VIEWPORT_DEPENDENT,
        ),
        feature!(
            Atom::from("orientation"),
            AllowsRanges::No,
            keyword_evaluator!(eval_orientation, Orientation),
            FeatureFlags::VIEWPORT_DEPENDENT,
        ),
        // The deprecated device-* features: the viewport is all the screen
        // this browser reports.
        feature!(Atom::from("device-width"), AllowsRanges::Yes, Evaluator::Length(eval_width), FeatureFlags::empty(),),
        feature!(Atom::from("device-height"), AllowsRanges::Yes, Evaluator::Length(eval_height), FeatureFlags::empty(),),
        feature!(
            Atom::from("device-aspect-ratio"),
            AllowsRanges::Yes,
            Evaluator::NumberRatio(eval_aspect_ratio),
            FeatureFlags::empty(),
        ),
        feature!(Atom::from("scan"), AllowsRanges::No, keyword_evaluator!(eval_scan, Scan), FeatureFlags::empty(),),
        feature!(Atom::from("resolution"), AllowsRanges::Yes, Evaluator::Resolution(eval_resolution), FeatureFlags::empty(),),
        feature!(
            Atom::from("device-pixel-ratio"),
            AllowsRanges::Yes,
            Evaluator::Float(eval_device_pixel_ratio),
            FeatureFlags::WEBKIT_PREFIX,
        ),
        feature!(
            Atom::from("-moz-device-pixel-ratio"),
            AllowsRanges::Yes,
            Evaluator::Float(eval_device_pixel_ratio),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("prefers-color-scheme"),
            AllowsRanges::No,
            keyword_evaluator!(eval_prefers_color_scheme, PrefersColorScheme),
            FeatureFlags::empty(),
        ),
        feature!(Atom::from("color"), AllowsRanges::Yes, Evaluator::Integer(eval_color), FeatureFlags::empty(),),
        feature!(Atom::from("color-index"), AllowsRanges::Yes, Evaluator::Integer(eval_zero), FeatureFlags::empty(),),
        feature!(Atom::from("monochrome"), AllowsRanges::Yes, Evaluator::Integer(eval_zero), FeatureFlags::empty(),),
        feature!(Atom::from("grid"), AllowsRanges::No, Evaluator::Integer(eval_zero), FeatureFlags::empty(),),
        feature!(Atom::from("hover"), AllowsRanges::No, keyword_evaluator!(eval_hover, Hover), FeatureFlags::empty(),),
        feature!(Atom::from("any-hover"), AllowsRanges::No, keyword_evaluator!(eval_hover, Hover), FeatureFlags::empty(),),
        feature!(Atom::from("pointer"), AllowsRanges::No, keyword_evaluator!(eval_pointer, Pointer), FeatureFlags::empty(),),
        feature!(Atom::from("any-pointer"), AllowsRanges::No, keyword_evaluator!(eval_pointer, Pointer), FeatureFlags::empty(),),
        feature!(
            Atom::from("prefers-reduced-motion"),
            AllowsRanges::No,
            keyword_evaluator!(eval_prefers_reduced_motion, PrefersReducedMotion),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("prefers-reduced-transparency"),
            AllowsRanges::No,
            keyword_evaluator!(eval_prefers_reduced_transparency, PrefersReducedTransparency),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("prefers-reduced-data"),
            AllowsRanges::No,
            keyword_evaluator!(eval_prefers_reduced_data, PrefersReducedData),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("prefers-contrast"),
            AllowsRanges::No,
            keyword_evaluator!(eval_prefers_contrast, PrefersContrast),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("forced-colors"),
            AllowsRanges::No,
            keyword_evaluator!(eval_forced_colors, ForcedColors),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("inverted-colors"),
            AllowsRanges::No,
            keyword_evaluator!(eval_inverted_colors, InvertedColors),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("dynamic-range"),
            AllowsRanges::No,
            keyword_evaluator!(eval_dynamic_range, DynamicRange),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("video-dynamic-range"),
            AllowsRanges::No,
            keyword_evaluator!(eval_dynamic_range, DynamicRange),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("color-gamut"),
            AllowsRanges::No,
            keyword_evaluator!(eval_color_gamut, ColorGamut),
            FeatureFlags::empty(),
        ),
        feature!(Atom::from("scripting"), AllowsRanges::No, keyword_evaluator!(eval_scripting, Scripting), FeatureFlags::empty(),),
        feature!(Atom::from("update"), AllowsRanges::No, keyword_evaluator!(eval_update, Update), FeatureFlags::empty(),),
        feature!(
            Atom::from("overflow-block"),
            AllowsRanges::No,
            keyword_evaluator!(eval_overflow_block, OverflowBlock),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("overflow-inline"),
            AllowsRanges::No,
            keyword_evaluator!(eval_overflow_inline, OverflowInline),
            FeatureFlags::empty(),
        ),
        feature!(
            Atom::from("display-mode"),
            AllowsRanges::No,
            keyword_evaluator!(eval_display_mode, DisplayMode),
            FeatureFlags::empty(),
        ),
    ]
});
