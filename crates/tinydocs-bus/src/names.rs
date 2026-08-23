//! `TinyDocs` bus identity and member names.

/// Well-known bus name exported by the `TinyDocs` module.
pub const BUS_NAME: &str = "ai.tinyhumans.tinydocs.Docx";

/// Object path served by the `TinyDocs` module.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinydocs/Docx";

/// One constant per method name on [`BUS_NAME`].
pub mod methods {
    /// `GenerateDocx` — generate a complete DOCX payload.
    pub const GENERATE_DOCX: &str = "GenerateDocx";
}

/// All method names in the declaration order used by the module interface.
pub const METHODS: [&str; 1] = [methods::GENERATE_DOCX];
