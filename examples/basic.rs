//! Generate a small `.docx` and report its size.
//!
//! Examples are compiled and linted in CI, so they cannot drift from the API.
//! Run it with:
//!
//! ```sh
//! cargo run --example basic
//! ```

use tinydocs::docx::{self, DocumentSection, DocumentSpec};

fn main() {
    let spec = DocumentSpec {
        title: "Quarterly Review".to_string(),
        author: Some("Ferris".to_string()),
        sections: vec![
            DocumentSection {
                heading: Some("Summary".to_string()),
                paragraphs: vec!["Throughput doubled while error rates fell.".to_string()],
                bullets: vec![],
            },
            DocumentSection {
                heading: Some("Next Quarter".to_string()),
                paragraphs: vec![],
                bullets: vec![
                    "Ship the streaming parser".to_string(),
                    "Halve p99 latency".to_string(),
                ],
            },
        ],
    };

    match docx::generate(&spec) {
        Ok(bytes) => println!("generated {} bytes of .docx", bytes.len()),
        Err(error) => println!("generation failed: {error}"),
    }

    // Failure modes are part of the public contract, so show one too. An empty
    // title is rejected before any synthesis happens.
    let invalid = DocumentSpec {
        title: "   ".to_string(),
        ..spec
    };
    match docx::generate(&invalid) {
        Ok(bytes) => println!("unexpectedly generated {} bytes", bytes.len()),
        Err(error) => println!("expected failure: {error}"),
    }
}
