//! The `.pptx` presentation spec: the typed description a caller hands to
//! `pptx::generate`, plus the size limits every spec is validated against.
//!
//! Same contract rules as [`crate::spec::document`] — `deny_unknown_fields`,
//! public limits, `validate` before synthesis — with one structural difference
//! worth understanding.
//!
//! # Images are bytes here, not references
//!
//! A [`SlideImage`] carries the image *bytes*, its format, and its native pixel
//! dimensions. It deliberately does **not** carry a path, a URL, or an
//! application-specific identifier, because resolving any of those is host
//! policy this crate has no business holding: which directories an agent may
//! read, whether a given identifier belongs to the caller, and whether fetching
//! a URL is an acceptable request to originate are all questions with different
//! answers in every host. A host resolves indirection under its own rules and
//! hands over the resulting bytes.
//!
//! [`SlideImage::from_bytes`] does the mechanical half of that hand-off:
//! identify the format and read the dimensions, or reject the bytes. It needs
//! no format writer, so a host can build and validate a whole spec in a build
//! with the `pptx` feature off.

use serde::{Deserialize, Serialize};

use crate::spec::image::ImageFormat;
use crate::{Error, Result};

/// Maximum number of content slides a single deck may contain.
///
/// Bounds generation time and output size; a caller with more material is
/// expected to split it across multiple decks.
pub const MAX_SLIDES: usize = 64;

/// Maximum length, in Unicode scalar values, of any single text field — the
/// deck title, the author byline, the theme hint, a slide title, a slide body,
/// one bullet, the speaker notes, or an image caption.
pub const MAX_TEXT_CHARS: usize = 2_000;

/// Maximum number of bullets on a single slide.
///
/// Higher counts produce a slide nobody can read, and bloat the output.
pub const MAX_BULLETS_PER_SLIDE: usize = 32;

/// Maximum number of images attached to a single slide.
///
/// The single-column layout stacks images vertically in the lower band of the
/// slide; past this count each one is too small to read.
pub const MAX_IMAGES_PER_SLIDE: usize = 6;

/// Maximum number of images across the whole deck.
///
/// Bounds the embedded media payload regardless of how the images are
/// distributed across slides.
pub const MAX_IMAGES_PER_DECK: usize = 8;

/// Maximum size, in bytes, of a single embedded image.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// One image embedded on a slide.
///
/// Construct with [`SlideImage::from_bytes`] rather than by hand: it derives
/// `format` and the dimensions from the bytes, which keeps the three fields
/// consistent by construction. [`PresentationSpec::validate`] re-checks that
/// consistency, because a spec can also arrive over a wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideImage {
    /// The encoded image, as PNG or JPEG bytes.
    pub bytes: Vec<u8>,
    /// The format of `bytes`.
    pub format: ImageFormat,
    /// Native width in pixels, used to place the image without distorting it.
    pub width_px: u32,
    /// Native height in pixels, used to place the image without distorting it.
    pub height_px: u32,
    /// Optional caption, rendered as a bullet beneath the image.
    #[serde(default)]
    pub caption: Option<String>,
}

impl SlideImage {
    /// Identify and measure `bytes`, producing a consistent [`SlideImage`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when `bytes` is empty, exceeds
    /// [`MAX_IMAGE_BYTES`], is not PNG or JPEG, or carries a header this crate
    /// cannot measure.
    pub fn from_bytes(bytes: Vec<u8>, caption: Option<String>) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::invalid_input("bytes", "must not be empty"));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::invalid_input(
                "bytes",
                format!("must be ≤ {MAX_IMAGE_BYTES} bytes"),
            ));
        }
        let format = ImageFormat::sniff(&bytes)
            .ok_or_else(|| Error::invalid_input("bytes", "must be a PNG or JPEG image"))?;
        let (width_px, height_px) = format.dimensions(&bytes).ok_or_else(|| {
            Error::invalid_input(
                "bytes",
                format!("{format} header is truncated or malformed"),
            )
        })?;
        Ok(Self {
            bytes,
            format,
            width_px,
            height_px,
            caption,
        })
    }
}

/// One content slide of the deck, rendered in spec order.
///
/// At least one of `title`, `body`, or `bullets` must carry renderable text.
/// Images alone are not enough — a slide holding only an image and no label
/// reads as a rendering bug rather than a design choice, and synthesis drops
/// blank text anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideSpec {
    /// Slide title. May be blank for a visually minimal slide, as long as the
    /// body or bullets carry text.
    #[serde(default)]
    pub title: String,
    /// Body text, rendered above the bullets. Plain text only.
    #[serde(default)]
    pub body: Option<String>,
    /// Bullets, rendered after the body text.
    #[serde(default)]
    pub bullets: Vec<String>,
    /// Speaker notes attached to the slide.
    #[serde(default)]
    pub speaker_notes: Option<String>,
    /// Images, stacked in a single column beneath the text.
    #[serde(default)]
    pub images: Vec<SlideImage>,
}

impl SlideSpec {
    /// Returns `true` when the slide carries no renderable text at all — the
    /// title, body, and every bullet are absent or blank.
    ///
    /// Synthesis trims and drops blank entries, so a slide holding only
    /// `["   "]` would render without text despite carrying entries.
    #[must_use]
    pub fn is_textless(&self) -> bool {
        let has_title = !self.title.trim().is_empty();
        let has_body = self.body.as_deref().is_some_and(|b| !b.trim().is_empty());
        let has_bullets = self.bullets.iter().any(|b| !b.trim().is_empty());
        !(has_title || has_body || has_bullets)
    }
}

/// A complete `.pptx` presentation spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationSpec {
    /// Deck title, rendered on a leading title slide. Required and non-blank.
    pub title: String,
    /// Optional author byline, rendered beneath the deck title.
    #[serde(default)]
    pub author: Option<String>,
    /// Optional theme hint.
    ///
    /// Accepted and validated but not yet acted on: synthesis uses the writer's
    /// default template regardless. It is part of the contract so a host's tool
    /// schema does not have to change when template selection lands.
    #[serde(default)]
    pub theme: Option<String>,
    /// Content slides, in display order. Must contain at least one entry.
    #[serde(default)]
    pub slides: Vec<SlideSpec>,
}

impl PresentationSpec {
    /// Total number of images across every slide.
    #[must_use]
    pub fn image_count(&self) -> usize {
        self.slides
            .iter()
            .map(|slide| slide.images.len())
            .sum::<usize>()
    }

    /// Check the spec against every documented size limit, and check that each
    /// image's declared format and dimensions match its bytes.
    ///
    /// Callers do not have to invoke this: `pptx::generate` validates before it
    /// synthesises anything. It is public so a host can reject a malformed spec
    /// at its own boundary — an LLM tool call, say — and hand back the
    /// structured [`Error::InvalidInput`] before paying for a blocking hop, a
    /// process boundary, or a bus round trip.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] naming the first field that violates a
    /// limit. Fields are checked in spec order, so the reported field is stable
    /// for a given spec.
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(Error::invalid_input("title", "must not be empty"));
        }
        Self::check_text_len("title", &self.title)?;
        if let Some(author) = self.author.as_deref() {
            Self::check_text_len("author", author)?;
        }
        if let Some(theme) = self.theme.as_deref() {
            Self::check_text_len("theme", theme)?;
        }
        if self.slides.is_empty() {
            return Err(Error::invalid_input(
                "slides",
                "must contain at least one slide",
            ));
        }
        if self.slides.len() > MAX_SLIDES {
            return Err(Error::invalid_input(
                "slides",
                format!("must contain ≤ {MAX_SLIDES} slides"),
            ));
        }
        // Checked across the whole deck rather than per slide: the per-slide cap
        // bounds readability, this one bounds the embedded media payload however
        // the images are distributed.
        if self.image_count() > MAX_IMAGES_PER_DECK {
            return Err(Error::invalid_input(
                "slides[].images",
                format!("deck must contain ≤ {MAX_IMAGES_PER_DECK} images total"),
            ));
        }

        for (i, slide) in self.slides.iter().enumerate() {
            if slide.is_textless() {
                return Err(Error::invalid_input(
                    format!("slides[{i}]"),
                    "must have at least one of title / body / bullets",
                ));
            }
            Self::check_text_len(format!("slides[{i}].title"), &slide.title)?;
            if let Some(body) = slide.body.as_deref() {
                Self::check_text_len(format!("slides[{i}].body"), body)?;
            }
            if slide.bullets.len() > MAX_BULLETS_PER_SLIDE {
                return Err(Error::invalid_input(
                    format!("slides[{i}].bullets"),
                    format!("must contain ≤ {MAX_BULLETS_PER_SLIDE} bullets"),
                ));
            }
            for (b, bullet) in slide.bullets.iter().enumerate() {
                Self::check_text_len(format!("slides[{i}].bullets[{b}]"), bullet)?;
            }
            if let Some(notes) = slide.speaker_notes.as_deref() {
                Self::check_text_len(format!("slides[{i}].speaker_notes"), notes)?;
            }
            if slide.images.len() > MAX_IMAGES_PER_SLIDE {
                return Err(Error::invalid_input(
                    format!("slides[{i}].images"),
                    format!("must contain ≤ {MAX_IMAGES_PER_SLIDE} images"),
                ));
            }
            for (m, image) in slide.images.iter().enumerate() {
                Self::check_image(&format!("slides[{i}].images[{m}]"), image)?;
            }
        }
        Ok(())
    }

    /// Reject a text field longer than [`MAX_TEXT_CHARS`] scalar values.
    fn check_text_len(field: impl Into<String>, value: &str) -> Result<()> {
        if value.chars().count() > MAX_TEXT_CHARS {
            return Err(Error::invalid_input(
                field,
                format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            ));
        }
        Ok(())
    }

    /// Re-derive an image's format and dimensions from its bytes and reject any
    /// disagreement with what the spec declares.
    ///
    /// [`SlideImage::from_bytes`] keeps the fields consistent by construction,
    /// but a spec can also arrive as deserialized JSON, where the three fields
    /// are independent. A declared format that does not match the bytes yields
    /// a part the reader refuses to render, and declared dimensions that do not
    /// match distort the image silently — both are worth a named rejection.
    fn check_image(field: &str, image: &SlideImage) -> Result<()> {
        if image.bytes.is_empty() {
            return Err(Error::invalid_input(
                format!("{field}.bytes"),
                "must not be empty",
            ));
        }
        if image.bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::invalid_input(
                format!("{field}.bytes"),
                format!("must be ≤ {MAX_IMAGE_BYTES} bytes"),
            ));
        }
        let sniffed = ImageFormat::sniff(&image.bytes).ok_or_else(|| {
            Error::invalid_input(format!("{field}.bytes"), "must be a PNG or JPEG image")
        })?;
        if sniffed != image.format {
            return Err(Error::invalid_input(
                format!("{field}.format"),
                format!("declared {} but the bytes are {sniffed}", image.format),
            ));
        }
        let (width_px, height_px) = sniffed.dimensions(&image.bytes).ok_or_else(|| {
            Error::invalid_input(
                format!("{field}.bytes"),
                format!("{sniffed} header is truncated or malformed"),
            )
        })?;
        if (width_px, height_px) != (image.width_px, image.height_px) {
            return Err(Error::invalid_input(
                format!("{field}.width_px"),
                format!(
                    "declared {}x{} but the bytes are {width_px}x{height_px}",
                    image.width_px, image.height_px
                ),
            ));
        }
        if let Some(caption) = image.caption.as_deref() {
            Self::check_text_len(format!("{field}.caption"), caption)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test;
