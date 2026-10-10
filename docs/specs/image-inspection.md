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

Presentation generation keeps its existing single concatenated image stream,
per-image bound, eight-image deck limit, and 40 MiB aggregate ceiling. This
operation streams one image at a time for callers that need facts before
assembling that existing request.

Hosts must use a compatible released module artifact before dispatching the
new member. Local module tests exercise PNG/JPEG fixtures and malformed input
through a loaded artifact and an in-memory broker; no external service is used.
