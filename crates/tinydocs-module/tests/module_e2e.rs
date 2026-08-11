//! End-to-end test for loading the built `TinyDocs` module into `TinyBus`.
//!
//! The only test that exercises the real thing: the built `cdylib`, the ABI
//! descriptor, manifest admission, the dynamic loader, and a broker routing
//! actual frames. Everything else in this crate calls Rust functions directly and
//! would keep passing if the artifact stopped loading altogether.
//!
//! It also carries the only honest test of the streaming paths. A stream needs
//! two connected peers with a broker between them, so a unit test against a bare
//! struct cannot reach one — and the payloads here are deliberately larger than a
//! single chunk, because a one-chunk transfer would not prove the reassembly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use base64::Engine as _;
use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinydocs::spec::{DocumentSection, DocumentSpec};
use tinydocs_module::{BUS_NAME, OBJECT_PATH, OutputRef, hex_digest};

/// Every method the manifest must declare, in order.
const EXPECTED_METHODS: &[&str] = &[
    "GenerateDocx",
    "GeneratePptx",
    "ExtractText",
    "ReadOutput",
    "ReleaseOutput",
];

/// Chunk size for reading outputs back.
///
/// Small on purpose so a modest document still takes several reads. The module's
/// own cap is megabytes; nothing here needs to approach it to prove the offsets
/// line up.
const READ_CHUNK: u64 = 512;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TINYDOCS_TEST_MODULE to point at the built cdylib"]
async fn the_built_module_serves_every_format_over_a_real_broker() {
    // One test rather than four: TinyBus never unloads a module and a second
    // load of the same artifact would collide on the well-known name, so every
    // format is exercised against the one admitted instance.
    let (client, modules, broker_task) = admit_module();
    let client = client.await;
    wait_until_serving(&client).await;

    let target = Target::new();
    let proxy = client.proxy(BUS_NAME, OBJECT_PATH, BUS_NAME).unwrap();

    generates_a_docx(&proxy).await;
    generates_a_pptx_from_a_streamed_image_pair(&client, &target, &proxy).await;
    extracts_text_from_a_streamed_pdf(&client, &target, &proxy).await;
    refuses_a_stream_that_contradicts_the_spec(&client, &target).await;

    assert!(matches!(modules.list()[0].state, ModuleState::Ready));
    broker_task.abort();
}

/// The destination triple every streaming call needs.
struct Target {
    destination: tinybus::BusName,
    path: tinybus::ObjectPath,
    interface: tinybus::InterfaceName,
}

impl Target {
    fn new() -> Self {
        Self {
            destination: tinybus::BusName::new(BUS_NAME).unwrap(),
            path: tinybus::ObjectPath::new(OBJECT_PATH).unwrap(),
            interface: tinybus::InterfaceName::new(BUS_NAME).unwrap(),
        }
    }
}

/// Load the built artifact and check its manifest against the interface.
fn admit_module() -> (
    impl std::future::Future<Output = Connection>,
    ModuleHost,
    tokio::task::JoinHandle<tinybus::Result<()>>,
) {
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

    let connect = async move {
        Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap()
    };
    (connect, modules, broker_task)
}

/// Wait for the module to claim its well-known name.
async fn wait_until_serving(client: &Connection) {
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
}

/// No inbound payload, a held document out.
async fn generates_a_docx(proxy: &tinybus::Proxy) {
    let handle: OutputRef = proxy
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
    let docx = download(proxy, &handle).await;
    assert_eq!(&docx[..2], b"PK", "a .docx is a zip container");

    // Releasing a document twice reports that it is gone rather than pretending.
    proxy
        .call::<()>("ReleaseOutput", (handle.output_id.clone(),))
        .await
        .expect("releasing a held document should succeed");
    proxy
        .call::<()>("ReleaseOutput", (handle.output_id,))
        .await
        .expect_err("releasing twice should fail");
}

/// Two images concatenated into one stream, split apart by their declared
/// lengths.
async fn generates_a_pptx_from_a_streamed_image_pair(
    client: &Connection,
    target: &Target,
    proxy: &tinybus::Proxy,
) {
    let first = png_padded_to(2_000);
    let second = png_padded_to(3_000);
    let (first_len, second_len) = (first.len(), second.len());
    let mut payload = first;
    payload.extend_from_slice(&second);

    let deck: OutputRef = client
        .call_with_stream(
            target.destination.clone(),
            target.path.clone(),
            target.interface.clone(),
            tinybus::MemberName::new("GeneratePptx").unwrap(),
            |stream| {
                serde_json::json!([
                    {
                        "title": "TinyBus E2E",
                        "slides": [{
                            "title": "With images",
                            "images": [
                                { "byte_len": first_len, "caption": "First" },
                                { "byte_len": second_len, "caption": "Second" },
                            ],
                        }],
                    },
                    stream,
                ])
            },
            &payload,
        )
        .await
        .expect("GeneratePptx should succeed");
    let pptx = download(proxy, &deck).await;
    assert_eq!(&pptx[..2], b"PK", "a .pptx is a zip container");
}

/// A streamed document in, extracted text out.
async fn extracts_text_from_a_streamed_pdf(
    client: &Connection,
    target: &Target,
    proxy: &tinybus::Proxy,
) {
    let pdf = pdf_with_text("Hello from the module");
    let extracted: OutputRef = client
        .call_with_stream(
            target.destination.clone(),
            target.path.clone(),
            target.interface.clone(),
            tinybus::MemberName::new("ExtractText").unwrap(),
            |stream| serde_json::json!([stream]),
            &pdf,
        )
        .await
        .expect("ExtractText should succeed");
    let text = String::from_utf8(download(proxy, &extracted).await).expect("text is utf-8");
    assert!(
        text.contains("Hello from the module"),
        "extracted text missing content: {text:?}"
    );
}

/// The lengths in the spec are the authority.
///
/// A short stream must fail rather than produce a deck with a picture assembled
/// from whatever bytes happened to arrive.
async fn refuses_a_stream_that_contradicts_the_spec(client: &Connection, target: &Target) {
    let mismatched: tinybus::Result<OutputRef> = client
        .call_with_stream(
            target.destination.clone(),
            target.path.clone(),
            target.interface.clone(),
            tinybus::MemberName::new("GeneratePptx").unwrap(),
            |stream| {
                serde_json::json!([
                    {
                        "title": "Mismatched",
                        "slides": [{
                            "title": "Truncated",
                            "images": [{ "byte_len": 9_999 }],
                        }],
                    },
                    stream,
                ])
            },
            b"too short",
        )
        .await;
    assert!(
        mismatched.is_err(),
        "a stream shorter than the declared images should be refused"
    );
}

/// Read a held document back in chunks and verify its digest.
async fn download(proxy: &tinybus::Proxy, handle: &OutputRef) -> Vec<u8> {
    let decoder = base64::engine::general_purpose::STANDARD;
    let mut out = Vec::with_capacity(usize::try_from(handle.total_bytes).unwrap_or_default());
    while (out.len() as u64) < handle.total_bytes {
        let encoded: String = proxy
            .call(
                "ReadOutput",
                (handle.output_id.clone(), out.len() as u64, READ_CHUNK),
            )
            .await
            .expect("ReadOutput should succeed");
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

/// A 1×1 PNG padded to roughly `total` bytes.
///
/// The padding rides in a `tEXt` chunk, which is ancillary — the file stays a
/// valid PNG the module will accept and measure.
fn png_padded_to(total: usize) -> Vec<u8> {
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

    // Chunk overhead for the tEXt chunk plus the IEND trailer that follows.
    let overhead = 12 + 4 + 12;
    let padding = total.saturating_sub(out.len() + overhead);
    let mut text_chunk = b"pad\0".to_vec();
    text_chunk.extend(std::iter::repeat_n(b'p', padding));
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
