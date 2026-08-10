//! Wire shapes that differ from the library spec because bytes cannot travel
//! inline.
//!
//! [`crate::blobs`] explains why: a `TinyBus` frame is a 16 MiB JSON document,
//! and a deck may legally carry 40 MiB of images. So on the bus an image is a
//! staged blob id, and the module resolves it into the real
//! [`tinydocs::spec::SlideImage`] — bytes, format and dimensions — after the
//! upload completes.
//!
//! Only the presentation spec needs this treatment. A document spec is text, and
//! its aggregate cap keeps it inside a frame, so `GenerateDocx` takes
//! [`tinydocs::spec::DocumentSpec`] unchanged.

use serde::{Deserialize, Serialize};

/// A slide image, as it appears on the bus: a reference to a staged blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireSlideImage {
    /// Id of a completed blob holding the PNG or JPEG bytes.
    pub blob_id: String,
    /// Optional caption, rendered as a bullet beneath the image.
    #[serde(default)]
    pub caption: Option<String>,
}

/// One content slide, as it appears on the bus.
///
/// Identical to [`tinydocs::spec::SlideSpec`] apart from `images`.
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
