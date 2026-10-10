# tinydocs-bus

The transport-free TinyBus contract for TinyDocs: its well-known bus identity,
member names, contract version, shared errors, and every document payload type
that crosses the bus boundary.

`tinydocs-bus` deliberately does not depend on `tinybus`, an async runtime, or
the OOXML writer. A host that only calls the module can depend on this crate to
construct `DocumentSpec` values without compiling the document implementation
or the loadable module. `tinydocs` depends on and re-exports the same types, so
`tinydocs::docx::DocumentSpec` and `tinydocs_bus::spec::DocumentSpec` are one
type, not compatible-looking duplicates.

The module serves the `METHODS` at `BUS_NAME` and `OBJECT_PATH`. Keep changes
here backward compatible or advance `CONTRACT_VERSION` according to the
documented compatibility rule.

Contract version 4 adds `InspectImage(StreamRef) -> ImageFacts` without changing
existing member arities. Contract version 3 added
`ConvertMarkdown(DocumentFormat, StreamRef) -> OutputRef`; see the
[conversion spec](../../docs/specs/markdown-conversion.md). Image facts are
shared vocabulary only; encoded image parsing belongs to `tinydocs`.
