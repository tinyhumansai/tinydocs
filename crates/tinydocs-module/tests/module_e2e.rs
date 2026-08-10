//! End-to-end test for loading the built `TinyDocs` module into `TinyBus`.
//!
//! This is the only test that exercises the real thing: the built `cdylib`, the
//! ABI descriptor, manifest admission, the dynamic loader, and a broker routing
//! actual frames. Everything else in this crate tests Rust functions directly and
//! would keep passing if the artifact stopped loading at all.
//!
//! It therefore covers each of the three formats end to end, and moves an image
//! across more than one chunk — the chunked path is the reason this interface
//! exists, and a single-chunk transfer would not prove it works.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinydocs::spec::{DocumentSection, DocumentSpec};
use tinydocs_module::{BUS_NAME, BlobRef, OBJECT_PATH, hex_digest};

/// Every method the manifest must declare.
const EXPECTED_METHODS: &[&str] = &[
    "BeginBlob",
    "PutChunk",
    "GetChunk",
    "ReleaseBlob",
    "GenerateDocx",
    "GeneratePptx",
    "ExtractText",
];

/// Chunk size used by the test transfers.
///
/// Deliberately small so a modest fixture still spans several chunks. The
/// module's own cap is megabytes; nothing here needs to approach it to prove the
/// offsets line up.
const TEST_CHUNK: usize = 512;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TINYDOCS_TEST_MODULE to point at the built cdylib"]
async fn the_built_module_serves_every_format_over_a_real_broker() {
    let artifact =
        std::env::var_os("TINYDOCS_TEST_MODULE").expect("TINYDOCS_TEST_MODULE must be set");
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let modules = ModuleHost::new(broker);

    let loaded = modules.load_file(artifact).expect("module should load");
    assert_eq!(loaded.name, "tinydocs-module");
    assert_eq!(loaded.manifest.bus_name.as_str(), BUS_NAME);
    assert_eq!(loaded.manifest.object_path.as_str(), OBJECT_PATH);

    let declared: Vec<&str> = loaded
        .manifest
        .provides
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .map(tinybus::MemberName::as_str)
        .collect();
    assert_eq!(
        declared, EXPECTED_METHODS,
        "manifest methods drifted from the interface"
    );

    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .list_names()
                .await
                .unwrap()
                .iter()
                .any(|name| name.as_str() == BUS_NAME)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("module should become ready");

    let proxy = client.proxy(BUS_NAME, OBJECT_PATH, BUS_NAME).unwrap();

    // --- .docx: text in, staged bytes out ---
    let handle: BlobRef = proxy
        .call(
            "GenerateDocx",
            (DocumentSpec {
                title: "TinyBus E2E".to_string(),
                author: Some("TinyDocs".to_string()),
                sections: vec![DocumentSection {
                    heading: Some("Loaded module".to_string()),
                    paragraphs: vec!["Generated through the real module ABI.".to_string()],
                    bullets: vec!["valid DOCX".to_string()],
                }],
            },),
        )
        .await
        .expect("GenerateDocx should succeed");
    let docx = download(&proxy, &handle).await;
    assert_eq!(&docx[..2], b"PK", "a .docx is a zip container");

    // --- .pptx: an image staged across several chunks, then a deck ---
    let png = png_1x1();
    assert!(
        png.len() > TEST_CHUNK,
        "the image fixture must span more than one chunk to be worth testing"
    );
    let image_blob = upload(&proxy, &png).await;
    let deck: BlobRef = proxy
        .call(
            "GeneratePptx",
            (serde_json::json!({
                "title": "TinyBus E2E",
                "slides": [{
                    "title": "With an image",
                    "images": [{ "blob_id": image_blob, "caption": "A chart" }],
                }],
            }),),
        )
        .await
        .expect("GeneratePptx should succeed");
    let pptx = download(&proxy, &deck).await;
    assert_eq!(&pptx[..2], b"PK", "a .pptx is a zip container");

    // --- .pdf: a staged document in, extracted text out ---
    let pdf = pdf_with_text("Hello from the module");
    let pdf_blob = upload(&proxy, &pdf).await;
    let extracted: BlobRef = proxy
        .call("ExtractText", (pdf_blob,))
        .await
        .expect("ExtractText should succeed");
    let text = String::from_utf8(download(&proxy, &extracted).await).expect("text is utf-8");
    assert!(
        text.contains("Hello from the module"),
        "extracted text missing content: {text:?}"
    );

    // Releasing a consumed handle is reported, not silently accepted.
    proxy
        .call::<()>("ReleaseBlob", (handle.blob_id.clone(),))
        .await
        .expect("releasing a staged output should succeed");
    proxy
        .call::<()>("ReleaseBlob", (handle.blob_id,))
        .await
        .expect_err("releasing twice should fail");

    assert!(matches!(modules.list()[0].state, ModuleState::Ready));
    broker_task.abort();
}

/// Stage `bytes` over `BeginBlob` + `PutChunk`, returning the blob id.
async fn upload(proxy: &tinybus::Proxy, bytes: &[u8]) -> String {
    use base64::Engine as _;
    let encoder = base64::engine::general_purpose::STANDARD;

    let blob_id: String = proxy
        .call("BeginBlob", (bytes.len() as u64, hex_digest(bytes)))
        .await
        .expect("BeginBlob should succeed");

    let mut offset = 0usize;
    while offset < bytes.len() {
        let end = (offset + TEST_CHUNK).min(bytes.len());
        let received: u64 = proxy
            .call(
                "PutChunk",
                (
                    blob_id.clone(),
                    offset as u64,
                    encoder.encode(&bytes[offset..end]),
                ),
            )
            .await
            .expect("PutChunk should succeed");
        assert_eq!(received, end as u64, "server disagreed about progress");
        offset = end;
    }
    blob_id
}

/// Read a staged blob back over `GetChunk` and verify its digest.
async fn download(proxy: &tinybus::Proxy, handle: &BlobRef) -> Vec<u8> {
    use base64::Engine as _;
    let decoder = base64::engine::general_purpose::STANDARD;

    let mut out = Vec::with_capacity(usize::try_from(handle.total_bytes).unwrap_or_default());
    while (out.len() as u64) < handle.total_bytes {
        let encoded: String = proxy
            .call(
                "GetChunk",
                (handle.blob_id.clone(), out.len() as u64, TEST_CHUNK as u64),
            )
            .await
            .expect("GetChunk should succeed");
        let chunk = decoder.decode(encoded).expect("chunk is base64");
        assert!(!chunk.is_empty(), "read stalled at offset {}", out.len());
        out.extend_from_slice(&chunk);
    }
    assert_eq!(
        hex_digest(&out),
        handle.sha256,
        "downloaded bytes do not match the declared digest"
    );
    out
}

/// A 1×1 PNG padded past [`TEST_CHUNK`] so its transfer spans several chunks.
///
/// The padding rides in a trailing comment chunk, which keeps the file a valid
/// PNG that the module will accept and measure.
fn png_1x1() -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&[0x08, 0x06, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    // tEXt is an ancillary chunk, so a reader that does not care skips it.
    let padding = vec![b'p'; TEST_CHUNK * 2];
    let mut text_chunk = b"pad\0".to_vec();
    text_chunk.extend_from_slice(&padding);
    out.extend_from_slice(&u32::try_from(text_chunk.len()).unwrap().to_be_bytes());
    out.extend_from_slice(b"tEXt");
    out.extend_from_slice(&text_chunk);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
    out
}

/// A valid single-page PDF whose text layer holds `text`.
fn pdf_with_text(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
