//! TinyBus module boundary for document synthesis.
//!
//! Enabling the `module` feature turns the crate's `cdylib` output into a
//! trusted in-process TinyBus module. The module owns no persistent state and
//! exposes one object: [`GenerateDocx`](TinyDocs::generate_docx) accepts the
//! same typed [`DocumentSpec`] as the Rust API and returns the complete DOCX
//! bytes.
//!
//! The TinyBus wire format has a 16 MiB frame limit. [`DocumentSpec`]'s
//! aggregate text limit keeps normal output comfortably below that boundary;
//! a larger future document format should use a path or file-descriptor based
//! transfer instead of increasing the bus frame cap.

use tinybus::{Connection, Error as BusError, Result as BusResult};

use crate::Error;
use crate::docx::{self, DocumentSpec};

/// Well-known name and interface exported by the TinyDocs module.
pub const BUS_NAME: &str = "ai.tinyhumans.tinydocs.Docx";

/// Object path exported by the TinyDocs module.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinydocs/Docx";

const INVALID_INPUT_ERROR: &str = "ai.tinyhumans.tinydocs.Error.InvalidInput";
const GENERATION_FAILED_ERROR: &str = "ai.tinyhumans.tinydocs.Error.GenerationFailed";

struct TinyDocs;

#[tinybus::interface(name = "ai.tinyhumans.tinydocs.Docx")]
impl TinyDocs {
    /// Generate a complete DOCX document from a validated specification.
    async fn generate_docx(&self, spec: DocumentSpec) -> BusResult<Vec<u8>> {
        tokio::task::spawn_blocking(move || docx::generate(&spec))
            .await
            .map_err(|_| BusError::MethodFailed {
                name: GENERATION_FAILED_ERROR.to_string(),
                message: "document generation worker failed".to_string(),
            })?
            .map_err(map_error)
    }
}

fn map_error(error: Error) -> BusError {
    let name = match error {
        Error::InvalidInput { .. } => INVALID_INPUT_ERROR,
        Error::GenerationFailed { .. } => GENERATION_FAILED_ERROR,
    };
    BusError::MethodFailed {
        name: name.to_string(),
        message: error.to_string(),
    }
}

async fn setup(connection: Connection) -> BusResult<()> {
    connection
        .serve_at(OBJECT_PATH.try_into()?, TinyDocs)
        .await?;
    connection.request_name(BUS_NAME).await?;
    Ok(())
}

tinybus_module::module_export! {
    setup = setup,
    worker_threads = 2,
    provides = ["ai.tinyhumans.tinydocs.Docx"],
    methods = ["GenerateDocx"],
    signals = [],
    requires = [],
    optional = [],
    lazy = false,
}

#[cfg(test)]
mod test;
