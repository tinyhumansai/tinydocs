# tinydocs-bus

The transport-free TinyBus contract for TinyDocs: its well-known bus identity,
member names, contract version, and the document payload types that cross the
bus boundary.

`tinydocs-bus` deliberately does not depend on `tinybus`, an async runtime, or
the OOXML writer. A host that only calls the module can depend on this crate to
construct `DocumentSpec` values without compiling the document implementation
or the loadable module. `tinydocs` depends on and re-exports the same types, so
`tinydocs::docx::DocumentSpec` and `tinydocs_bus::docx::DocumentSpec` are one
type, not compatible-looking duplicates.

The module serves `GenerateDocx(DocumentSpec) -> Vec<u8>` at [`BUS_NAME`] and
[`OBJECT_PATH`]. Keep changes here backward compatible or advance
[`CONTRACT_VERSION`] according to the documented compatibility rule.
