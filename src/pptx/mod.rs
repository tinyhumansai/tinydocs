//! `.pptx` (OOXML `PresentationML`) synthesis, backed by
//! [`ppt-rs`](https://crates.io/crates/ppt-rs).
//!
//! [`generate`] turns a validated [`PresentationSpec`] into the bytes of a
//! `.pptx` file. Like [`crate::docx::generate`] it is **synchronous, pure, and
//! CPU-bound**: it touches no filesystem, spawns no subprocess, and knows
//! nothing about deadlines. A host that needs a timeout or a blocking-pool hop
//! owns that policy and wraps this call.
//!
//! # Spec → `PresentationML` mapping
//!
//! `ppt_rs::SlideContent` has no separate body-paragraph slot: everything below
//! the title is a bullet. [`SlideSpec::body`] therefore collapses into a leading
//! bullet, so body text still reaches the rendered slide:
//!
//! ```text
//! SlideSpec { title, body: Some(b), bullets: [b1, b2], speaker_notes: Some(n) }
//!   → SlideContent::new(title).add_bullet(b).add_bullet(b1).add_bullet(b2).notes(n)
//! ```
//!
//! Blank and whitespace-only entries are dropped rather than emitting an empty
//! bullet marker.
//!
//! # The title slide is synthetic
//!
//! `ppt_rs::create_pptx_with_content(title, slides)` treats `title` as deck
//! metadata only — it lands in `docProps/core.xml` and does **not** produce a
//! title slide. A deck built from it would open straight onto the first content
//! slide. [`generate`] therefore prepends a slide carrying
//! [`PresentationSpec::title`] and the optional author byline, which is why the
//! rendered deck holds one more slide than `spec.slides.len()`.
//!
//! # Image layout
//!
//! Images stack in a single vertical column in the lower band of the slide,
//! beneath the text. Each is scaled to fit its slot with its aspect ratio
//! preserved and is centred in both axes; a slot is never upscaled past the
//! source's natural size ratio. Every dimension below is in EMU (English Metric
//! Units, 914,400 per inch), the unit OOXML itself uses.

// The spec is defined in `crate::spec`, which is compiled in every build so a
// host can share the wire contract without the OOXML writer stack. Re-exported
// here so `tinydocs::pptx::PresentationSpec` names the same type.
pub use crate::spec::image::ImageFormat;
pub use crate::spec::presentation::{
    MAX_BULLETS_PER_SLIDE, MAX_IMAGE_BYTES, MAX_IMAGES_PER_DECK, MAX_IMAGES_PER_SLIDE, MAX_SLIDES,
    MAX_TEXT_CHARS, PresentationSpec, SlideImage, SlideSpec,
};

use ppt_rs::generator::{Image, SlideContent, create_pptx_with_content};

use crate::{Error, Result};

/// Slide width in EMU — 10 inches, matching the writer's default 4:3 deck.
const SLIDE_WIDTH_EMU: u32 = 9_144_000;
/// Slide height in EMU — 7.5 inches.
const SLIDE_HEIGHT_EMU: u32 = 6_858_000;
/// Left and right margin in EMU — 1 inch, leaving the usable content column.
const SIDE_MARGIN_EMU: u32 = 914_400;
/// Top of the image band in EMU, roughly the slide midpoint: images live below
/// the title/body placeholder rather than over it.
const IMAGE_BAND_TOP_EMU: u32 = 3_429_000;
/// Margin in EMU kept below the image band — half an inch.
const IMAGE_BAND_BOTTOM_MARGIN_EMU: u32 = 457_200;
/// Vertical gap in EMU between two stacked images.
const IMAGE_STACK_GAP_EMU: u32 = 91_440;
/// EMU per pixel at 96 DPI, matching the writer's own px→EMU convention.
const EMU_PER_PX: u32 = 9_525;

/// Validate `spec` and synthesise it into `.pptx` bytes.
///
/// The returned buffer is a complete OOXML zip container: any reader compatible
/// with `PowerPoint` can open it, and a host can write it straight to disk or
/// stream it.
///
/// Synchronous and CPU-bound. A deck at the slide cap completes well under a
/// second, but a host on an async executor should still run this on a blocking
/// pool rather than inline.
///
/// The rendered deck carries `spec.slides.len() + 1` slides: the extra one is
/// the synthetic title slide described in the module docs.
///
/// # Errors
///
/// - [`Error::InvalidInput`] if `spec` violates any documented limit, or if an
///   image's declared format or dimensions contradict its bytes — no synthesis
///   is attempted.
/// - [`Error::GenerationFailed`] if `ppt-rs` fails to pack the deck.
///
/// # Examples
///
/// ```
/// use tinydocs::pptx::{generate, PresentationSpec, SlideSpec};
///
/// let spec = PresentationSpec {
///     title: "Quarterly Review".to_string(),
///     author: Some("Alice".to_string()),
///     theme: None,
///     slides: vec![SlideSpec {
///         title: "Highlights".to_string(),
///         body: Some("Throughput doubled.".to_string()),
///         bullets: vec!["Shipped the parser".to_string()],
///         speaker_notes: Some("Mention the benchmark.".to_string()),
///         images: vec![],
///     }],
/// };
///
/// let bytes = generate(&spec)?;
/// assert_eq!(&bytes[0..2], b"PK", "a .pptx is a zip container");
/// # Ok::<(), tinydocs::Error>(())
/// ```
pub fn generate(spec: &PresentationSpec) -> Result<Vec<u8>> {
    spec.validate()?;
    create_pptx_with_content(&spec.title, build_slides(spec))
        // The writer's error type is not guaranteed to be `Send + Sync +
        // 'static`, so it is rendered to text at the boundary rather than
        // carried.
        .map_err(|err| Error::generation_failed(&format!("{err}")))
}

/// Pure transformation from the spec to the writer's slide model.
///
/// Split out from [`generate`] for unit-testability: the slide ordering and
/// blank-filtering rules are load-bearing for the rendered deck shape.
fn build_slides(spec: &PresentationSpec) -> Vec<SlideContent> {
    let mut out = Vec::with_capacity(spec.slides.len() + 1);

    // The synthetic title slide. See the module docs: without it the deck would
    // open on the first content slide and the title would only reach core.xml.
    let mut title_slide = SlideContent::new(&spec.title);
    if let Some(author) = spec.author.as_deref().filter(|a| !a.trim().is_empty()) {
        title_slide = title_slide.add_bullet(author);
    }
    out.push(title_slide);

    for slide in &spec.slides {
        let mut built = SlideContent::new(&slide.title);
        if let Some(body) = slide.body.as_deref().filter(|b| !b.trim().is_empty()) {
            built = built.add_bullet(body);
        }
        for bullet in &slide.bullets {
            if !bullet.trim().is_empty() {
                built = built.add_bullet(bullet);
            }
        }
        // Images go on after the text, so a caption bullet lands beneath the
        // image it labels rather than above the body.
        for placed in place_single_column(&slide.images) {
            built = built.add_image(placed.image);
            if let Some(caption) = placed.caption {
                built = built.add_bullet(&caption);
            }
        }
        if let Some(notes) = slide
            .speaker_notes
            .as_deref()
            .filter(|n| !n.trim().is_empty())
        {
            built = built.notes(notes);
        }
        out.push(built);
    }

    out
}

/// A positioned image plus the caption that labels it.
struct PlacedImage {
    image: Image,
    caption: Option<String>,
}

/// Lay `images` out in a single vertical column inside the slide's lower band.
///
/// The band is divided into equal slots with a fixed gap between them. Each
/// image is scaled to fit its slot with its aspect ratio preserved, then centred
/// horizontally and vertically inside it.
fn place_single_column(images: &[SlideImage]) -> Vec<PlacedImage> {
    // Saturating rather than fallible: `MAX_IMAGES_PER_SLIDE` is 6, so a slice
    // long enough to overflow a `u32` cannot come from a validated spec. If one
    // ever did, saturating collapses every slot to zero height and the images
    // degenerate visibly instead of panicking.
    let count = u32::try_from(images.len()).unwrap_or(u32::MAX);
    if count == 0 {
        return Vec::new();
    }

    let content_left = SIDE_MARGIN_EMU;
    let content_width = SLIDE_WIDTH_EMU.saturating_sub(2 * SIDE_MARGIN_EMU);
    let band_height = SLIDE_HEIGHT_EMU
        .saturating_sub(IMAGE_BAND_TOP_EMU)
        .saturating_sub(IMAGE_BAND_BOTTOM_MARGIN_EMU);
    let total_gap = IMAGE_STACK_GAP_EMU.saturating_mul(count.saturating_sub(1));
    let slot_height = band_height.saturating_sub(total_gap) / count;

    // The slot top advances by one stride per image. Accumulating it beats
    // multiplying by the index: no `usize`-to-`u32` conversion of the index, and
    // the stride is stated once.
    let stride = slot_height.saturating_add(IMAGE_STACK_GAP_EMU);
    let mut slot_top = IMAGE_BAND_TOP_EMU;

    images
        .iter()
        .map(|img| {
            let this_slot_top = slot_top;
            slot_top = slot_top.saturating_add(stride);
            let (width, height) = fit_within(
                img.width_px.saturating_mul(EMU_PER_PX),
                img.height_px.saturating_mul(EMU_PER_PX),
                content_width,
                slot_height,
            );
            let x = content_left + content_width.saturating_sub(width) / 2;
            let y = this_slot_top + slot_height.saturating_sub(height) / 2;
            PlacedImage {
                image: Image::from_bytes(img.bytes.clone(), width, height, img.format.as_str())
                    .position(x, y),
                caption: img.caption.clone(),
            }
        })
        .collect()
}

/// Scale `(w, h)` to fit inside `(max_w, max_h)` with the aspect ratio preserved.
///
/// Both inputs and outputs are EMU. A degenerate zero dimension falls back to
/// the bounding box, because there is no ratio to preserve; validation rejects
/// zero-dimension images before synthesis, so this is a guard rather than a
/// path. The result is clamped into the box and never below 1 EMU, since a
/// zero-extent image is a part the reader rejects.
///
/// The arithmetic is integer throughout, in `u64`. Scaling a dimension by a
/// floating-point factor and rounding back would be the obvious spelling, but it
/// makes the result depend on binary64 rounding for no benefit at this
/// magnitude: an EMU is 1/914,400 inch, so a one-unit difference is invisible,
/// and the intermediate products here (a dimension times a slide extent) top out
/// near 4×10^16, comfortably inside `u64`.
fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (max_w, max_h);
    }
    let (w, h) = (u64::from(w), u64::from(h));
    let (box_w, box_h) = (u64::from(max_w), u64::from(max_h));

    // Fit to the width first. If the resulting height overflows the box, the
    // height is the binding constraint instead, so fit to that.
    let height_at_full_width = div_round(h * box_w, w);
    let (fit_w, fit_h) = if height_at_full_width <= box_h {
        (box_w, height_at_full_width)
    } else {
        (div_round(w * box_h, h), box_h)
    };

    (clamp_into_box(fit_w, max_w), clamp_into_box(fit_h, max_h))
}

/// `numerator / denominator`, rounded to nearest rather than truncated.
///
/// # Panics
///
/// Panics if `denominator` is zero. Both call sites guard against a zero
/// dimension before reaching here.
fn div_round(numerator: u64, denominator: u64) -> u64 {
    (numerator + denominator / 2) / denominator
}

/// Clamp a computed extent into `1..=max`, narrowing to `u32`.
///
/// `value` is always a scaled dimension bounded by `max`, so the narrowing
/// cannot lose information; `unwrap_or` states the fallback rather than
/// asserting the invariant with a panic.
fn clamp_into_box(value: u64, max: u32) -> u32 {
    u32::try_from(value).unwrap_or(max).clamp(1, max.max(1))
}

#[cfg(test)]
mod test;
