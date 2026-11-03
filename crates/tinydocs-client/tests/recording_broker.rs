#![expect(
    clippy::expect_used,
    reason = "fixture assertions should stop immediately on broken broker setup"
)]

//! Product-facing `TinyDocs` client tests over an in-process recording broker.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tinybus::broker::Broker;
use tinybus::module::{
    AccessFuture, FailureReport, FailureReporter, FailureSink, ModuleAccess, ModuleClient,
    ProxyFailure, ReleaseRecord,
};
use tinybus::service::Interface;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Error, InterfaceName, MemberName, ObjectPath, Proxy};
use tinydocs_bus::names::{BUS_NAME, METHODS, OBJECT_PATH, methods};
use tinydocs_bus::{DocumentFormat, DocumentSpec, ExtractDocumentSpec, OutputRef, RenderPdfSpec};
use tinydocs_client::{DocumentCallError, TinyDocsClient};
use tokio::sync::Notify;

const MODULE_ID: &str = "tinydocs";
const CHUNK_BYTES: u64 = 1024 * 1024;
const CHUNK_BYTES_USIZE: usize = 1024 * 1024;
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

static RECORD: ReleaseRecord = ReleaseRecord {
    id: MODULE_ID,
    bus_name: BUS_NAME,
    object_path: OBJECT_PATH,
    version: "0.2.5",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.5",
    assets: &[],
};

#[derive(Default)]
struct NoopSink;

impl FailureSink for NoopSink {
    fn capture(&self, _report: &FailureReport<'_>) {}
}

struct FixtureAccess {
    connection: Connection,
    reporter: FailureReporter,
    enabled: bool,
}

impl ModuleAccess for FixtureAccess {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        (id == MODULE_ID).then_some(&RECORD)
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn reporter(&self) -> &FailureReporter {
        &self.reporter
    }

    fn ensure_loaded<'a>(&'a self, _id: &'a str) -> AccessFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn proxy<'a>(
        &'a self,
        bus_name: &'a str,
        object_path: &'a str,
    ) -> AccessFuture<'a, Result<Proxy, ProxyFailure>> {
        Box::pin(async move {
            self.connection
                .proxy(bus_name, object_path, bus_name)
                .map_err(|_| ProxyFailure::Contract)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    member: String,
    args: Value,
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    input_streams: BTreeMap<String, Vec<u8>>,
    outputs: HashMap<String, Vec<u8>>,
    output_overrides: HashMap<String, Vec<u8>>,
    generated_bytes: Vec<u8>,
    next_output: usize,
    faults: HashMap<String, (String, String)>,
    malformed_replies: HashMap<String, Value>,
    declared_size: Option<u64>,
    declared_digest: Option<String>,
    read_mode: ReadMode,
    block_reads: bool,
    read_started: Arc<Notify>,
    release_seen: Arc<Notify>,
    resume_read: Arc<Notify>,
    release_started: Arc<Notify>,
    release_delay: Option<Duration>,
    handover_to: Option<Connection>,
    detach_on_read: bool,
}

#[derive(Clone, Copy, Default)]
enum ReadMode {
    #[default]
    Normal,
    Empty,
    InvalidBase64,
    Overlong,
}

#[derive(Clone)]
struct RecordingService {
    connection: Connection,
    state: Arc<Mutex<State>>,
}

impl RecordingService {
    fn hold(&self, bytes: Vec<u8>) -> OutputRef {
        let mut state = self.state.lock().expect("fixture state lock");
        state.next_output += 1;
        let output_id = format!("output-{}", state.next_output);
        let sha256 = state
            .declared_digest
            .clone()
            .unwrap_or_else(|| digest(&bytes));
        let total_bytes = state.declared_size.unwrap_or(bytes.len() as u64);
        state.outputs.insert(output_id.clone(), bytes);
        OutputRef {
            output_id,
            total_bytes,
            sha256,
        }
    }

    fn hold_for(&self, member: &str, fallback: &[u8]) -> OutputRef {
        let bytes = self
            .state
            .lock()
            .expect("fixture state lock")
            .output_overrides
            .remove(member)
            .unwrap_or_else(|| fallback.to_vec());
        self.hold(bytes)
    }

    async fn read_stream_arg(
        &self,
        args: &Value,
        index: usize,
        member: &str,
    ) -> tinybus::Result<()> {
        let stream: tinybus::stream::StreamRef = serde_json::from_value(args[index].clone())?;
        let bytes = self.connection.read_stream(&stream).await?;
        self.state
            .lock()
            .expect("fixture state lock")
            .input_streams
            .insert(member.to_string(), bytes);
        Ok(())
    }

    fn record_call(&self, member: &str, args: &Value) -> Option<(String, String)> {
        let mut state = self.state.lock().expect("fixture state lock");
        state.calls.push(Call {
            member: member.to_string(),
            args: args.clone(),
        });
        state.faults.get(member).cloned()
    }

    fn malformed_reply(&self, member: &str) -> Option<Value> {
        self.state
            .lock()
            .expect("fixture state lock")
            .malformed_replies
            .get(member)
            .cloned()
    }

    async fn return_fault(
        &self,
        member: &str,
        args: &Value,
        (name, message): (String, String),
    ) -> tinybus::Result<Value> {
        let stream_index = match member {
            methods::GENERATE_PPTX if !args[1].is_null() => Some(1),
            methods::EXTRACT_TEXT | methods::INSPECT_IMAGE => Some(0),
            methods::EXTRACT_DOCUMENT | methods::CONVERT_MARKDOWN | methods::RENDER_PDF => Some(1),
            _ => None,
        };
        if let Some(index) = stream_index {
            self.read_stream_arg(args, index, member).await?;
        }
        Err(Error::MethodFailed { name, message })
    }

    async fn return_malformed_reply(
        &self,
        member: &str,
        args: &Value,
        reply: Value,
    ) -> tinybus::Result<Value> {
        if member == methods::EXTRACT_DOCUMENT {
            self.read_stream_arg(args, 1, member).await?;
        }
        Ok(reply)
    }

    async fn dispatch_member(&self, member: &str, args: Value) -> tinybus::Result<Value> {
        match member {
            methods::GENERATE_DOCX => {
                let bytes = self
                    .state
                    .lock()
                    .expect("fixture state lock")
                    .generated_bytes
                    .clone();
                let output = self.hold(bytes);
                self.handover_alias().await?;
                Ok(serde_json::to_value(output)?)
            }
            methods::GENERATE_PPTX => {
                if !args[1].is_null() {
                    self.read_stream_arg(&args, 1, member).await?;
                }
                let output = self.hold_for(member, b"pptx bytes");
                self.handover_alias().await?;
                Ok(serde_json::to_value(output)?)
            }
            methods::EXTRACT_TEXT => {
                self.read_stream_arg(&args, 0, member).await?;
                let output = self.hold_for(member, b"extracted text");
                self.handover_alias().await?;
                Ok(serde_json::to_value(output)?)
            }
            methods::EXTRACT_DOCUMENT => {
                self.read_stream_arg(&args, 1, member).await?;
                Ok(json!({
                    "format": "pdf",
                    "section_count": 1,
                    "sections": [{
                        "source": "page:1",
                        "index": 1,
                        "text": "section text",
                        "scanned_candidate": false
                    }],
                    "truncated": false
                }))
            }
            methods::CONVERT_MARKDOWN => {
                self.read_stream_arg(&args, 1, member).await?;
                let output = self.hold_for(member, b"# converted\n");
                self.handover_alias().await?;
                Ok(serde_json::to_value(output)?)
            }
            methods::INSPECT_IMAGE => {
                self.read_stream_arg(&args, 0, member).await?;
                Ok(json!({ "format": "PNG", "width_px": 2, "height_px": 3 }))
            }
            methods::RENDER_PDF => {
                self.read_stream_arg(&args, 1, member).await?;
                let first = self.hold(b"page one".to_vec());
                let second = self.hold(b"page two".to_vec());
                self.handover_alias().await?;
                Ok(json!({
                    "page_count": 3,
                    "pages": [
                        { "page": 1, "width": 2, "height": 3, "output": first },
                        { "page": 3, "width": 4, "height": 5, "output": second }
                    ]
                }))
            }
            methods::READ_OUTPUT => self.read_output(&args).await,
            methods::RELEASE_OUTPUT => {
                let output_id = args[0].as_str().expect("output id string");
                let (release_delay, release_started) = {
                    let state = self.state.lock().expect("fixture state lock");
                    (state.release_delay, state.release_started.clone())
                };
                if let Some(delay) = release_delay {
                    release_started.notify_one();
                    tokio::time::sleep(delay).await;
                }
                self.state
                    .lock()
                    .expect("fixture state lock")
                    .outputs
                    .remove(output_id);
                self.state
                    .lock()
                    .expect("fixture state lock")
                    .release_seen
                    .notify_one();
                Ok(Value::Null)
            }
            unknown => Err(Error::MethodFailed {
                name: "ai.tinyhumans.tinybus.Error.UnknownMethod".into(),
                message: format!("unexpected test member {unknown}"),
            }),
        }
    }
}

#[async_trait]
impl Interface for RecordingService {
    fn name(&self) -> InterfaceName {
        InterfaceName::new(BUS_NAME).expect("valid TinyDocs interface")
    }

    fn members(&self) -> Vec<MemberName> {
        METHODS
            .iter()
            .map(|member| MemberName::new(*member).expect("valid TinyDocs member"))
            .collect()
    }

    async fn call(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        let member = member.as_str().to_string();
        let fault = self.record_call(&member, &args);
        if let Some((name, message)) = fault {
            return self.return_fault(&member, &args, (name, message)).await;
        }
        if let Some(reply) = self.malformed_reply(&member) {
            return self.return_malformed_reply(&member, &args, reply).await;
        }
        self.dispatch_member(&member, args).await
    }
}

impl RecordingService {
    async fn handover_alias(&self) -> tinybus::Result<()> {
        let replacement = self
            .state
            .lock()
            .expect("fixture state lock")
            .handover_to
            .take();
        if let Some(replacement) = replacement {
            self.connection.release_name(BUS_NAME).await?;
            replacement.request_name(BUS_NAME).await?;
        }
        Ok(())
    }

    async fn read_output(&self, args: &Value) -> tinybus::Result<Value> {
        let output_id = args[0].as_str().expect("output id string").to_string();
        let offset = args[1].as_u64().expect("output offset");
        let requested = args[2].as_u64().expect("requested length");
        let (mode, block, started, resume, detach, bytes) = {
            let state = self.state.lock().expect("fixture state lock");
            (
                state.read_mode,
                state.block_reads,
                state.read_started.clone(),
                state.resume_read.clone(),
                state.detach_on_read,
                state.outputs.get(&output_id).cloned().unwrap_or_default(),
            )
        };
        if detach {
            self.connection.close().await?;
        }
        if block {
            started.notify_one();
            resume.notified().await;
        }
        let value = match mode {
            ReadMode::Empty => String::new(),
            ReadMode::InvalidBase64 => "!".to_string(),
            ReadMode::Normal | ReadMode::Overlong => {
                let start = usize::try_from(offset).unwrap_or(usize::MAX);
                let wanted = usize::try_from(requested).unwrap_or(usize::MAX);
                let extra = usize::from(matches!(mode, ReadMode::Overlong));
                let end = start
                    .saturating_add(wanted)
                    .saturating_add(extra)
                    .min(bytes.len());
                BASE64.encode(bytes.get(start..end).unwrap_or_default())
            }
        };
        Ok(Value::String(value))
    }
}

struct Fixture {
    client: TinyDocsClient,
    state: Arc<Mutex<State>>,
    _broker: tokio::task::JoinHandle<tinybus::Result<()>>,
}

async fn test_fixture() -> Fixture {
    test_fixture_with_options(true, None).await
}

async fn test_fixture_with_enabled(enabled: bool) -> Fixture {
    test_fixture_with_options(enabled, None).await
}

async fn test_fixture_with_timeout(timeout: Duration) -> Fixture {
    test_fixture_with_options(true, Some(timeout)).await
}

async fn test_fixture_with_options(enabled: bool, timeout: Option<Duration>) -> Fixture {
    let bus = MemoryBus::new();
    let broker = Broker::new().spawn(bus.clone());
    let server = Connection::connect(bus.connect().await.expect("server transport"))
        .await
        .expect("server connection");
    let initial_state = State {
        generated_bytes: b"docx bytes".to_vec(),
        ..State::default()
    };
    let state = Arc::new(Mutex::new(initial_state));
    server
        .serve_at(
            ObjectPath::new(OBJECT_PATH).expect("valid TinyDocs path"),
            RecordingService {
                connection: server.clone(),
                state: state.clone(),
            },
        )
        .await
        .expect("serve recording module");
    server
        .request_name(BUS_NAME)
        .await
        .expect("claim module name");

    let client_connection = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client connection");
    let proxy = client_connection
        .proxy(BUS_NAME, OBJECT_PATH, BUS_NAME)
        .expect("fixture proxy");
    let mut sdk_client = ModuleClient::new(FixtureAccess {
        connection: client_connection.clone(),
        reporter: FailureReporter::new(Arc::new(NoopSink)),
        enabled,
    })
    .with_fixture(proxy);
    if let Some(timeout) = timeout {
        sdk_client = sdk_client.with_timeout(timeout);
    }

    Fixture {
        client: TinyDocsClient::new(sdk_client),
        state,
        _broker: broker,
    }
}

struct PeerHandoverFixture {
    client: TinyDocsClient,
    original: Arc<Mutex<State>>,
    replacement: Arc<Mutex<State>>,
    _broker: tokio::task::JoinHandle<tinybus::Result<()>>,
}

async fn peer_handover_fixture() -> PeerHandoverFixture {
    let bus = MemoryBus::new();
    let broker = Broker::new().spawn(bus.clone());
    let original_connection = Connection::connect(bus.connect().await.expect("original transport"))
        .await
        .expect("original connection");
    let replacement_connection =
        Connection::connect(bus.connect().await.expect("replacement transport"))
            .await
            .expect("replacement connection");
    let mut original_state = State {
        generated_bytes: b"original bytes".to_vec(),
        handover_to: Some(replacement_connection.clone()),
        ..State::default()
    };
    original_state.output_overrides.insert(
        methods::GENERATE_PPTX.into(),
        b"original pptx bytes".to_vec(),
    );
    let mut replacement_state = State {
        generated_bytes: b"replacement bytes".to_vec(),
        ..State::default()
    };
    replacement_state.output_overrides.insert(
        methods::GENERATE_PPTX.into(),
        b"replacement pptx bytes".to_vec(),
    );
    original_state
        .output_overrides
        .insert(methods::EXTRACT_TEXT.into(), b"original text".to_vec());
    replacement_state
        .output_overrides
        .insert(methods::EXTRACT_TEXT.into(), b"replacement text".to_vec());
    original_state.output_overrides.insert(
        methods::CONVERT_MARKDOWN.into(),
        b"original markdown\n".to_vec(),
    );
    replacement_state.output_overrides.insert(
        methods::CONVERT_MARKDOWN.into(),
        b"replacement markdown\n".to_vec(),
    );
    let original = Arc::new(Mutex::new(original_state));
    let replacement = Arc::new(Mutex::new(replacement_state));
    for (connection, state) in [
        (&original_connection, original.clone()),
        (&replacement_connection, replacement.clone()),
    ] {
        connection
            .serve_at(
                ObjectPath::new(OBJECT_PATH).expect("valid TinyDocs path"),
                RecordingService {
                    connection: connection.clone(),
                    state,
                },
            )
            .await
            .expect("serve peer");
    }
    original_connection
        .request_name(BUS_NAME)
        .await
        .expect("original owns alias");
    let client_connection = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client connection");
    let access = FixtureAccess {
        connection: client_connection,
        reporter: FailureReporter::new(Arc::new(NoopSink)),
        enabled: true,
    };
    PeerHandoverFixture {
        client: TinyDocsClient::new(ModuleClient::new(access).with_timeout(Duration::from_secs(2))),
        original,
        replacement,
        _broker: broker,
    }
}

fn digest(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing into String");
    }
    output
}

fn document_spec() -> DocumentSpec {
    DocumentSpec {
        title: "Broker proof".into(),
        author: None,
        sections: vec![],
    }
}

#[tokio::test]
async fn held_docx_uses_the_peer_that_owns_its_generation() {
    let fixture = peer_handover_fixture().await;
    let result = fixture.client.generate_docx(&document_spec()).await;
    let replacement_control = fixture.client.generate_docx(&document_spec()).await;

    // The producer transfers the well-known name only after retaining its
    // output. The later read and release must still reach that same peer.
    let original_bytes = result
        .as_ref()
        .is_ok_and(|bytes| bytes == b"original bytes");
    let replacement_bytes = replacement_control
        .as_ref()
        .is_ok_and(|bytes| bytes == b"replacement bytes");
    let original = fixture.original.lock().expect("original state lock");
    let replacement = fixture.replacement.lock().expect("replacement state lock");
    let original_counts = (
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::GENERATE_DOCX)
            .count(),
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::READ_OUTPUT)
            .count(),
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
    );
    let replacement_counts = (
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::GENERATE_DOCX)
            .count(),
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::READ_OUTPUT)
            .count(),
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
    );
    assert!(
        original_bytes
            && replacement_bytes
            && original_counts == (1, 1, 1)
            && replacement_counts == (1, 1, 1),
        "peer-hop DOCX: original_bytes={original_bytes}, replacement_control_bytes={replacement_bytes}, original generate/read/release={original_counts:?}, replacement generate/read/release={replacement_counts:?}"
    );
}

#[tokio::test]
async fn held_pptx_upload_and_output_use_the_peer_that_owns_generation() {
    let fixture = peer_handover_fixture().await;
    let deck = tinydocs_bus::WirePresentationSpec {
        title: "Deck".into(),
        author: None,
        theme: None,
        slides: vec![],
    };
    let images = b"fresh image bytes";
    let result = fixture.client.generate_pptx(&deck, images).await;
    let replacement_control = fixture.client.generate_pptx(&deck, images).await;

    let original_bytes = result
        .as_ref()
        .is_ok_and(|bytes| bytes == b"original pptx bytes");
    let replacement_bytes = replacement_control
        .as_ref()
        .is_ok_and(|bytes| bytes == b"replacement pptx bytes");
    let original = fixture.original.lock().expect("original state lock");
    let replacement = fixture.replacement.lock().expect("replacement state lock");
    let original_counts = (
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::GENERATE_PPTX)
            .count(),
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::READ_OUTPUT)
            .count(),
        original
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
    );
    let replacement_counts = (
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::GENERATE_PPTX)
            .count(),
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::READ_OUTPUT)
            .count(),
        replacement
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
    );
    let original_stream = original
        .input_streams
        .get(methods::GENERATE_PPTX)
        .map(Vec::as_slice)
        == Some(images.as_slice());
    let replacement_stream = replacement
        .input_streams
        .get(methods::GENERATE_PPTX)
        .map(Vec::as_slice)
        == Some(images.as_slice());
    assert!(
        original_bytes
            && replacement_bytes
            && original_counts == (1, 1, 1)
            && replacement_counts == (1, 1, 1)
            && original_stream
            && replacement_stream,
        "peer-hop PPTX: original_bytes={original_bytes}, replacement_control_bytes={replacement_bytes}, original generate/read/release={original_counts:?}, replacement generate/read/release={replacement_counts:?}, original_stream={original_stream}, replacement_stream={replacement_stream}"
    );
}

fn output_call_counts(state: &State, producer: &str) -> (usize, usize, usize) {
    (
        state
            .calls
            .iter()
            .filter(|call| call.member == producer)
            .count(),
        state
            .calls
            .iter()
            .filter(|call| call.member == methods::READ_OUTPUT)
            .count(),
        state
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
    )
}

#[tokio::test]
async fn streamed_text_outputs_stay_with_their_producer_after_alias_handover() {
    for (member, original_text, replacement_text) in [
        (methods::EXTRACT_TEXT, "original text", "replacement text"),
        (
            methods::CONVERT_MARKDOWN,
            "original markdown\n",
            "replacement markdown\n",
        ),
    ] {
        let fixture = peer_handover_fixture().await;
        let first = if member == methods::EXTRACT_TEXT {
            fixture.client.extract_text(b"PDF input").await
        } else {
            fixture
                .client
                .convert_markdown(DocumentFormat::Pdf, b"PDF input")
                .await
        };
        let replacement_control = if member == methods::EXTRACT_TEXT {
            fixture.client.extract_text(b"PDF input").await
        } else {
            fixture
                .client
                .convert_markdown(DocumentFormat::Pdf, b"PDF input")
                .await
        };
        let original_bytes = first.as_ref().is_ok_and(|text| text == original_text);
        let replacement_bytes = replacement_control
            .as_ref()
            .is_ok_and(|text| text == replacement_text);
        let original = fixture.original.lock().expect("original state lock");
        let replacement = fixture.replacement.lock().expect("replacement state lock");
        let original_counts = output_call_counts(&original, member);
        let replacement_counts = output_call_counts(&replacement, member);
        assert!(
            original_bytes
                && replacement_bytes
                && original_counts == (1, 1, 1)
                && replacement_counts == (1, 1, 1),
            "peer-hop {member}: original_bytes={original_bytes}, replacement_control_bytes={replacement_bytes}, original producer/read/release={original_counts:?}, replacement producer/read/release={replacement_counts:?}"
        );
    }
}

#[tokio::test]
async fn multi_page_render_outputs_stay_with_their_producer_after_alias_handover() {
    let fixture = peer_handover_fixture().await;
    let spec = RenderPdfSpec {
        pages: vec![1, 3],
        max_dimension: 256,
        max_total_pixels: 65_536,
        max_output_bytes: 1_048_576,
    };
    let first = fixture.client.render_pdf(b"PDF input", &spec).await;
    let replacement_control = fixture.client.render_pdf(b"PDF input", &spec).await;
    let expected = vec![(1, b"page one".to_vec()), (3, b"page two".to_vec())];
    let original_pages = first.as_ref().is_ok_and(|pages| pages == &expected);
    let replacement_pages = replacement_control
        .as_ref()
        .is_ok_and(|pages| pages == &expected);
    let original = fixture.original.lock().expect("original state lock");
    let replacement = fixture.replacement.lock().expect("replacement state lock");
    let original_counts = output_call_counts(&original, methods::RENDER_PDF);
    let replacement_counts = output_call_counts(&replacement, methods::RENDER_PDF);
    assert!(
        original_pages
            && replacement_pages
            && original_counts == (1, 2, 2)
            && replacement_counts == (1, 2, 2),
        "peer-hop RenderPdf: original_pages={original_pages}, replacement_control_pages={replacement_pages}, original render/read/release={original_counts:?}, replacement render/read/release={replacement_counts:?}"
    );
}

#[tokio::test]
async fn detached_output_owner_does_not_fall_through_to_alias_replacement() {
    let fixture = peer_handover_fixture().await;
    fixture
        .original
        .lock()
        .expect("original state lock")
        .detach_on_read = true;
    // The replacement deliberately has the same opaque resource id and bytes.
    // A mistaken alias retry could therefore appear successful without the
    // per-peer call counters below.
    fixture
        .replacement
        .lock()
        .expect("replacement state lock")
        .outputs
        .insert("output-1".into(), b"original bytes".to_vec());

    let result = fixture.client.generate_docx(&document_spec()).await;
    assert!(matches!(result, Err(DocumentCallError::Failed(_))));
    let original = fixture.original.lock().expect("original state lock");
    let replacement = fixture.replacement.lock().expect("replacement state lock");
    assert_eq!(
        output_call_counts(&original, methods::GENERATE_DOCX),
        (1, 1, 0)
    );
    assert!(
        replacement.calls.is_empty(),
        "detached output work must not reach the alias replacement"
    );
}

#[tokio::test]
async fn typed_operations_use_contract_members_and_stream_document_bytes() {
    let fixture = test_fixture().await;
    let input = b"incoming PDF bytes";

    let generated = fixture
        .client
        .generate_docx(&document_spec())
        .await
        .expect("held DOCX should be assembled");
    assert_eq!(generated, b"docx bytes");

    let deck = tinydocs_bus::WirePresentationSpec {
        title: "Deck".into(),
        author: None,
        theme: None,
        slides: vec![],
    };
    let slide_images = b"slide image bytes";
    assert_eq!(
        fixture
            .client
            .generate_pptx(&deck, slide_images)
            .await
            .expect("PPTX output is assembled"),
        b"pptx bytes"
    );
    assert_eq!(
        fixture
            .client
            .generate_pptx(&deck, &[])
            .await
            .expect("text-only PPTX avoids an upload stream"),
        b"pptx bytes"
    );
    assert_eq!(
        fixture
            .client
            .extract_text(input)
            .await
            .expect("text is read"),
        "extracted text"
    );
    assert_eq!(
        fixture
            .client
            .convert_markdown(DocumentFormat::Pdf, input)
            .await
            .expect("Markdown is read"),
        "# converted\n"
    );

    let extracted = fixture
        .client
        .extract_document(input, &ExtractDocumentSpec::new(DocumentFormat::Pdf))
        .await
        .expect("typed extraction succeeds");
    assert_eq!(extracted.sections[0].text, "section text");

    let image = fixture
        .client
        .inspect_image(b"PNG stream")
        .await
        .expect("image facts are decoded");
    assert_eq!((image.width_px, image.height_px), (2, 3));

    let state = fixture.state.lock().expect("fixture state lock");
    assert_eq!(state.input_streams[methods::EXTRACT_DOCUMENT], input);
    assert_eq!(state.input_streams[methods::INSPECT_IMAGE], b"PNG stream");
    assert_eq!(state.input_streams[methods::GENERATE_PPTX], slide_images);
    assert_eq!(state.input_streams[methods::EXTRACT_TEXT], input);
    assert_eq!(state.input_streams[methods::CONVERT_MARKDOWN], input);
    assert!(state.calls.iter().any(|call| {
        call.member == methods::GENERATE_DOCX && call.args[0]["title"] == "Broker proof"
    }));
    assert!(
        state
            .calls
            .iter()
            .any(|call| { call.member == methods::GENERATE_PPTX && call.args[1].is_null() })
    );
}

#[tokio::test]
async fn disabled_modules_are_unavailable_and_local_image_bounds_send_no_call() {
    let fixture = test_fixture_with_enabled(false).await;
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::Unavailable(_))
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .calls
            .len(),
        0
    );

    let fixture = test_fixture().await;
    assert!(matches!(
        fixture.client.inspect_image(&[]).await,
        Err(DocumentCallError::InvalidInput(_))
    ));
    let oversized = vec![0; tinydocs_bus::spec::presentation::MAX_IMAGE_BYTES + 1];
    assert!(matches!(
        fixture.client.inspect_image(&oversized).await,
        Err(DocumentCallError::InvalidInput(_))
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .calls
            .len(),
        0
    );
}

#[tokio::test]
async fn text_outputs_must_be_utf8_and_release_errors_do_not_discard_good_bytes() {
    for member in [methods::EXTRACT_TEXT, methods::CONVERT_MARKDOWN] {
        let fixture = test_fixture().await;
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .output_overrides
            .insert(member.to_string(), vec![0xff]);
        let result = if member == methods::EXTRACT_TEXT {
            fixture.client.extract_text(b"PDF bytes").await.map(|_| ())
        } else {
            fixture
                .client
                .convert_markdown(DocumentFormat::Pdf, b"PDF bytes")
                .await
                .map(|_| ())
        };
        assert!(matches!(result, Err(DocumentCallError::Failed(_))));
        {
            let state = fixture.state.lock().expect("fixture state lock");
            assert!(state.outputs.is_empty());
            assert!(
                state
                    .calls
                    .iter()
                    .any(|call| call.member == methods::RELEASE_OUTPUT)
            );
        }
    }

    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::RELEASE_OUTPUT.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.UnknownOutput".into(),
                "already expired".into(),
            ),
        );
    assert_eq!(
        fixture
            .client
            .generate_docx(&document_spec())
            .await
            .expect("a release failure does not invalidate collected bytes"),
        b"docx bytes"
    );
}

#[tokio::test(start_paused = true)]
async fn output_release_waits_for_the_module_clients_configured_deadline() {
    let fixture = test_fixture_with_timeout(Duration::from_secs(10)).await;
    let release_started = fixture
        .state
        .lock()
        .expect("fixture state lock")
        .release_started
        .clone();
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .release_delay = Some(Duration::from_secs(6));

    let client = fixture.client.clone();
    let call = tokio::spawn(async move { client.generate_docx(&document_spec()).await });
    release_started.notified().await;
    tokio::time::advance(Duration::from_millis(5_100)).await;
    tokio::task::yield_now().await;
    assert!(
        !call.is_finished(),
        "ordinary output release must honor the configured ten-second module deadline"
    );

    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        call.await
            .expect("client task joins")
            .expect("output is collected"),
        b"docx bytes"
    );
    assert!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .outputs
            .is_empty()
    );
}

#[tokio::test]
async fn credential_like_normal_invalid_input_is_redacted_from_formatted_errors() {
    let fixture = test_fixture().await;
    let remote_text = "credential-like-token-DoNotDisplay";
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::GENERATE_DOCX.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.InvalidInput".into(),
                remote_text.into(),
            ),
        );
    let error = fixture
        .client
        .generate_docx(&document_spec())
        .await
        .expect_err("invalid input is returned");
    assert!(matches!(error, DocumentCallError::InvalidInput(_)));
    assert_eq!(error.raw_message(), remote_text);
    assert!(!error.to_string().contains(remote_text));
    assert!(!format!("{error:?}").contains(remote_text));
}

#[tokio::test]
async fn credential_like_held_output_error_is_redacted_from_formatted_errors() {
    let fixture = test_fixture().await;
    let remote_text = "credential-like-output-token-DoNotDisplay";
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::READ_OUTPUT.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.InvalidInput".into(),
                remote_text.into(),
            ),
        );
    let error = fixture
        .client
        .generate_docx(&document_spec())
        .await
        .expect_err("held output error is returned");
    assert!(matches!(error, DocumentCallError::InvalidInput(_)));
    assert_eq!(error.raw_message(), remote_text);
    assert!(!error.to_string().contains(remote_text));
    assert!(!format!("{error:?}").contains(remote_text));
    assert!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .outputs
            .is_empty()
    );
}

#[tokio::test]
async fn ordinary_module_wire_failures_keep_input_availability_and_execution_distinct() {
    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::GENERATE_DOCX.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.InvalidInput".into(),
                "title is empty".into(),
            ),
        );
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::InvalidInput(_))
    ));

    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::GENERATE_DOCX.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.GenerationFailed".into(),
                "private generator detail".into(),
            ),
        );
    assert!(matches!(
        fixture
            .client
            .generate_docx(&document_spec())
            .await
            .expect_err("generation failure is returned"),
        DocumentCallError::Failed(_)
    ));

    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::GENERATE_DOCX.to_string(),
            (
                "ai.tinyhumans.tinybus.Error.ModuleUnavailable".into(),
                "module is down".into(),
            ),
        );
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::Unavailable(_))
    ));
}

#[tokio::test]
async fn streamed_module_wire_failures_keep_input_and_execution_distinct() {
    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::EXTRACT_DOCUMENT.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.InvalidInput".into(),
                "PDF header is invalid".into(),
            ),
        );
    assert!(matches!(
        fixture
            .client
            .extract_document(
                b"invalid PDF",
                &ExtractDocumentSpec::new(DocumentFormat::Pdf)
            )
            .await,
        Err(DocumentCallError::InvalidInput(_))
    ));

    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::INSPECT_IMAGE.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.ExtractionFailed".into(),
                "image parser failed".into(),
            ),
        );
    assert!(matches!(
        fixture.client.inspect_image(b"PNG bytes").await,
        Err(DocumentCallError::Failed(_))
    ));
}

#[tokio::test]
async fn held_output_invalid_input_is_classified_and_still_released() {
    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .faults
        .insert(
            methods::READ_OUTPUT.to_string(),
            (
                "ai.tinyhumans.tinydocs.Error.InvalidInput".into(),
                "output range is invalid".into(),
            ),
        );
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::InvalidInput(_))
    ));
    let released = {
        let state = fixture.state.lock().expect("fixture state lock");
        state
            .calls
            .iter()
            .any(|call| call.member == methods::RELEASE_OUTPUT)
            && state.outputs.is_empty()
    };
    assert!(released);
}

#[tokio::test]
async fn malformed_typed_stream_reply_is_unavailable_for_contract_mismatch() {
    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .malformed_replies
        .insert(
            methods::EXTRACT_DOCUMENT.to_string(),
            json!({"truncated": "wrong type"}),
        );
    let error = fixture
        .client
        .extract_document(b"PDF bytes", &ExtractDocumentSpec::new(DocumentFormat::Pdf))
        .await
        .expect_err("a malformed typed reply is rejected");
    assert!(matches!(error, DocumentCallError::Unavailable(_)));
}

#[tokio::test]
async fn held_outputs_are_read_in_order_and_released_after_success() {
    let fixture = test_fixture().await;
    let bytes = vec![0x5a; CHUNK_BYTES_USIZE * 2 + 13];
    {
        let mut state = fixture.state.lock().expect("fixture state lock");
        state.generated_bytes = bytes.clone();
        state.declared_digest = Some(digest(&bytes));
    }
    let got = fixture
        .client
        .generate_docx(&document_spec())
        .await
        .expect("output is assembled");
    assert_eq!(got, bytes);
    let offsets = fixture
        .state
        .lock()
        .expect("fixture state lock")
        .calls
        .iter()
        .filter(|call| call.member == methods::READ_OUTPUT)
        .map(|call| call.args[1].as_u64().expect("read offset"))
        .collect::<Vec<_>>();
    assert_eq!(offsets, [0, CHUNK_BYTES, CHUNK_BYTES * 2]);
    assert!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .outputs
            .is_empty()
    );
}

#[tokio::test]
async fn held_outputs_with_bad_digest_or_declared_size_are_released() {
    let fixture = test_fixture().await;
    {
        let mut state = fixture.state.lock().expect("fixture state lock");
        state.generated_bytes = b"digest must reject this output".to_vec();
        state.declared_digest = Some("0".repeat(64));
    }
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::Failed(_))
    ));
    {
        let state = fixture.state.lock().expect("fixture state lock");
        assert!(
            state
                .calls
                .iter()
                .any(|call| call.member == methods::RELEASE_OUTPUT)
        );
        assert!(state.outputs.is_empty());
    }

    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .declared_size = Some(MAX_DOCUMENT_BYTES + 1);
    assert!(matches!(
        fixture.client.generate_docx(&document_spec()).await,
        Err(DocumentCallError::Failed(_))
    ));
    {
        let state = fixture.state.lock().expect("fixture state lock");
        assert!(
            !state
                .calls
                .iter()
                .any(|call| call.member == methods::READ_OUTPUT)
        );
        assert!(
            state
                .calls
                .iter()
                .any(|call| call.member == methods::RELEASE_OUTPUT)
        );
    }
}

#[tokio::test]
async fn truncated_invalid_and_overlong_chunks_are_released() {
    for mode in [ReadMode::Empty, ReadMode::InvalidBase64, ReadMode::Overlong] {
        let fixture = test_fixture().await;
        {
            let mut state = fixture.state.lock().expect("fixture state lock");
            state.read_mode = mode;
            if matches!(mode, ReadMode::Overlong) {
                state.generated_bytes = vec![0x33; CHUNK_BYTES_USIZE + 16];
                state.declared_size = Some(CHUNK_BYTES);
                let declared_digest = digest(&state.generated_bytes);
                state.declared_digest = Some(declared_digest);
            }
        }
        assert!(matches!(
            fixture.client.generate_docx(&document_spec()).await,
            Err(DocumentCallError::Failed(_))
        ));
        assert!(
            fixture
                .state
                .lock()
                .expect("fixture state lock")
                .calls
                .iter()
                .any(|call| call.member == methods::RELEASE_OUTPUT)
        );
    }
}

#[tokio::test]
async fn cancellation_releases_held_output_and_render_releases_every_page() {
    let fixture = test_fixture().await;
    fixture
        .state
        .lock()
        .expect("fixture state lock")
        .block_reads = true;
    let started = fixture
        .state
        .lock()
        .expect("fixture state lock")
        .read_started
        .clone();
    let release_seen = fixture
        .state
        .lock()
        .expect("fixture state lock")
        .release_seen
        .clone();
    let client = fixture.client.clone();
    let call = tokio::spawn(async move { client.generate_docx(&document_spec()).await });
    started.notified().await;
    call.abort();
    let _ = call.await;
    release_seen.notified().await;
    let resume_read = fixture
        .state
        .lock()
        .expect("fixture state lock")
        .resume_read
        .clone();
    resume_read.notify_one();
    assert!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .outputs
            .is_empty()
    );

    let fixture = test_fixture().await;
    let rendered = fixture
        .client
        .render_pdf(
            b"PDF input",
            &RenderPdfSpec {
                pages: vec![1, 3],
                max_dimension: 256,
                max_total_pixels: 65_536,
                max_output_bytes: 1_048_576,
            },
        )
        .await
        .expect("selected pages are collected");
    assert_eq!(
        rendered.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert!(
        fixture
            .state
            .lock()
            .expect("fixture state lock")
            .outputs
            .is_empty()
    );

    let fixture = test_fixture().await;
    fixture.state.lock().expect("fixture state lock").read_mode = ReadMode::InvalidBase64;
    assert!(matches!(
        fixture
            .client
            .render_pdf(
                b"PDF input",
                &RenderPdfSpec {
                    pages: vec![1, 3],
                    max_dimension: 256,
                    max_total_pixels: 65_536,
                    max_output_bytes: 1_048_576,
                },
            )
            .await,
        Err(DocumentCallError::Failed(_))
    ));
    let state = fixture.state.lock().expect("fixture state lock");
    assert!(state.outputs.is_empty());
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|call| call.member == methods::RELEASE_OUTPUT)
            .count(),
        2
    );
}
