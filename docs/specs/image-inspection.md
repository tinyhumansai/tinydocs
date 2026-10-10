# Image inspection over TinyBus

Contract version 4 adds `InspectImage(StreamRef) -> ImageFacts`. Existing
members keep their names, wire representations, and argument counts.

The caller streams one encoded image to the module. The module admits no more
than the existing 5 MiB per-image limit, identifies PNG or JPEG bytes, and
returns only the shared `ImageFacts { format, width_px, height_px }` value.
Empty, unsupported, truncated, malformed, and zero-dimension images return the
existing `InvalidInput` wire error. A transfer failure remains
`TransferFailed`.

Image parsing belongs to the TinyDocs implementation, not the transport-free
`tinydocs-bus` crate. The contract contains only the format enum, image facts,
and existing size limits. `PresentationSpec::validate` checks structural
fields and limits; the implementation checks encoded headers before writing a
presentation.

This ownership move relocates the public source APIs that previously parsed
encoded bytes on `ImageFormat` and `SlideImage`. Migrate parser callers to
`tinydocs::image::inspect(bytes)` or `tinydocs::image::slide_image_from_bytes`
when linking the implementation. A host that depends only on `tinydocs-bus`
can call `InspectImage` with a bounded stream, then construct `SlideImage`
from the returned `ImageFacts` and the original bytes. The serialized image
facts, `SlideImage` fields, and existing bus member argument counts remain
unchanged. Since the bus crate is pre-1.0, release packaging for this source
API change must advance the workspace package version to 0.2; this source
change leaves package versions to the release workflow.

Presentation generation keeps its existing single concatenated image stream,
per-image bound, eight-image deck limit, and 40 MiB aggregate ceiling. This
operation streams one image at a time for callers that need facts before
assembling that existing request.

Hosts must use a compatible released module artifact before dispatching the
new member. Local module tests exercise PNG/JPEG fixtures and malformed input
through a loaded artifact and an in-memory broker; no external service is used.
