//! End-to-end test for loading the built `TinyDocs` module into `TinyBus`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinydocs::docx::{DocumentSection, DocumentSpec};
use tinydocs_module::{BUS_NAME, OBJECT_PATH};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TINYDOCS_TEST_MODULE to point at the built cdylib"]
async fn built_cdylib_loads_and_generates_a_docx_over_the_bus() {
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
    assert!(
        loaded
            .manifest
            .provides
            .iter()
            .flat_map(|interface| interface.methods.iter())
            .any(|method| method.as_str() == "GenerateDocx")
    );

    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
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
    let bytes: Vec<u8> = proxy
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
        .expect("bus call should succeed");

    assert_eq!(&bytes[..2], b"PK");
    assert!(matches!(modules.list()[0].state, ModuleState::Ready));
    broker_task.abort();
}
