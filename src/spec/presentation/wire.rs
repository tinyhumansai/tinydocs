//! The presentation spec as it crosses a bus, where bytes cannot travel inline.
//!
//! A `TinyBus` frame is a 16 MiB JSON document and a deck may legally carry
//! 40 MiB of images, so image bytes ride a stream beside the call rather than
//! inside it. A call has one stream and a deck has many images, so the images
//! are concatenated in slide order and each one declares its `byte_len`; the
//! module splits them apart and resolves each into a real
//! [`super::SlideImage`] — bytes, format and dimensions.
//!
//! The lengths live in the spec rather than in the stream because they are what
//! makes a truncated or over-long transfer a named rejection instead of a deck
//! with a picture assembled from two different images.
//!
//! Only the presentation spec needs this treatment. A document spec is text and
//! its aggregate cap keeps it inside a frame, so a document crosses unchanged.
//!
//! Defined here rather than in the module that serves it so a host driving that
//! module over a bus shares one definition of the shape instead of re-declaring
//! it. Like the rest of [`crate::spec`] it is serde and nothing else.

use serde::{Deserialize, Serialize};

/// A slide image, as it appears on the bus: a reference to a staged blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSlideImage {
    /// Length of this image's bytes within the concatenated image stream.
    pub byte_len: u64,
    /// Optional caption, rendered as a bullet beneath the image.
    #[serde(default)]
    pub caption: Option<String>,
}

/// One content slide, as it appears on the bus.
///
/// Identical to [`super::SlideSpec`] apart from `images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSlideSpec {
    /// Slide title.
    #[serde(default)]
    pub title: String,
    /// Body text, rendered above the bullets.
    #[serde(default)]
    pub body: Option<String>,
    /// Bullets, rendered after the body text.
    #[serde(default)]
    pub bullets: Vec<String>,
    /// Speaker notes attached to the slide.
    #[serde(default)]
    pub speaker_notes: Option<String>,
    /// Images, each naming a staged blob.
    #[serde(default)]
    pub images: Vec<WireSlideImage>,
}

/// A deck, as it appears on the bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WirePresentationSpec {
    /// Deck title, rendered on a leading title slide.
    pub title: String,
    /// Optional author byline.
    #[serde(default)]
    pub author: Option<String>,
    /// Optional theme hint.
    #[serde(default)]
    pub theme: Option<String>,
    /// Content slides, in display order.
    #[serde(default)]
    pub slides: Vec<WireSlideSpec>,
}
