//! Unit tests for the produced-document store.
//!
//! Weighted towards refusals and expiry. Holding bytes and handing them back is
//! the easy half; what decides whether a module that is never unloaded leaks is
//! the four bounds and the TTL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{
    IDLE_TTL, MAX_CHUNK_BYTES, MAX_LIVE_OUTPUTS, MAX_OUTPUT_BYTES, MAX_TOTAL_BYTES, OutputError,
    OutputStore, hex_digest,
};

fn t0() -> Instant {
    Instant::now()
}

#[test]
fn an_output_round_trips_through_chunks() {
    let store = OutputStore::new();
    let now = t0();
    let document: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();

    let handle = store.insert(document.clone(), now).unwrap();
    assert_eq!(handle.total_bytes, document.len() as u64);
    assert_eq!(handle.sha256, hex_digest(&document));

    let mut read = Vec::new();
    while (read.len() as u64) < handle.total_bytes {
        let chunk = store
            .read_chunk(&handle.output_id, read.len() as u64, 3_000, now)
            .unwrap();
        assert!(!chunk.is_empty(), "read stalled at {}", read.len());
        read.extend_from_slice(&chunk);
    }
    assert_eq!(read, document);
}

#[test]
fn a_read_past_the_end_is_clamped_not_refused() {
    // So a caller can ask for a whole chunk on the final read.
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(b"twelve bytes".to_vec(), now).unwrap();
    let chunk = store
        .read_chunk(&handle.output_id, 6, 1_000_000, now)
        .unwrap();
    assert_eq!(chunk, b" bytes");
}

#[test]
fn a_read_starting_past_the_end_is_refused() {
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(b"short".to_vec(), now).unwrap();
    assert_eq!(
        store.read_chunk(&handle.output_id, 99, 10, now),
        Err(OutputError::ReadPastEnd)
    );
}

#[test]
fn an_oversize_read_is_refused() {
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(b"small".to_vec(), now).unwrap();
    assert_eq!(
        store.read_chunk(&handle.output_id, 0, MAX_CHUNK_BYTES as u64 + 1, now),
        Err(OutputError::ChunkTooLarge)
    );
    assert_eq!(
        store.read_chunk(&handle.output_id, 0, u64::MAX, now),
        Err(OutputError::ChunkTooLarge)
    );
}

#[test]
fn an_oversize_document_is_refused() {
    let store = OutputStore::new();
    assert_eq!(
        store.insert(vec![0u8; MAX_OUTPUT_BYTES + 1], t0()),
        Err(OutputError::OutputTooLarge)
    );
    assert_eq!(store.live_count(), 0);
}

#[test]
fn the_total_budget_is_enforced() {
    let store = OutputStore::new();
    let now = t0();
    let half = MAX_TOTAL_BYTES / 2;
    // Two at the per-output cap fill the store, because the per-output cap is
    // half the total.
    store.insert(vec![1u8; half], now).unwrap();
    store.insert(vec![2u8; half], now).unwrap();
    assert_eq!(
        store.insert(vec![3u8; 16], now),
        Err(OutputError::StoreFull)
    );
}

#[test]
fn too_many_live_outputs_is_refused() {
    let store = OutputStore::new();
    let now = t0();
    for _ in 0..MAX_LIVE_OUTPUTS {
        store.insert(b"x".to_vec(), now).unwrap();
    }
    assert_eq!(store.live_count(), MAX_LIVE_OUTPUTS);
    assert_eq!(
        store.insert(b"one more".to_vec(), now),
        Err(OutputError::TooManyOutputs)
    );
}

#[test]
fn an_unread_output_expires_and_frees_its_budget() {
    // The bound that matters: a caller that asks for a document and then dies
    // must not cost the host that document for the life of the process.
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(vec![7u8; 1_000], now).unwrap();

    // Alive right on the boundary.
    let at_ttl = now + IDLE_TTL;
    assert!(store.read_chunk(&handle.output_id, 0, 10, at_ttl).is_ok());

    // Past it, measured from the last read, the next operation sweeps it away.
    let past_ttl = at_ttl + IDLE_TTL + Duration::from_secs(1);
    assert_eq!(
        store.read_chunk(&handle.output_id, 0, 10, past_ttl),
        Err(OutputError::UnknownOutput)
    );
    assert_eq!(store.live_count(), 0);
}

#[test]
fn reading_keeps_a_slow_consumer_alive() {
    // The flip side: a caller working through a large document in chunks must
    // never have it reaped mid-read.
    let store = OutputStore::new();
    let mut now = t0();
    let document = vec![4u8; 400];
    let handle = store.insert(document.clone(), now).unwrap();

    let mut read = Vec::new();
    while (read.len() as u64) < handle.total_bytes {
        // Each read lands just inside the window; four of them sum to well past
        // the TTL.
        now += IDLE_TTL.saturating_sub(Duration::from_secs(1));
        let chunk = store
            .read_chunk(&handle.output_id, read.len() as u64, 100, now)
            .expect("a document being read must not expire");
        read.extend_from_slice(&chunk);
    }
    assert_eq!(read, document);
}

#[test]
fn releasing_frees_the_budget_and_is_reported_once() {
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(b"payload".to_vec(), now).unwrap();
    assert!(store.release(&handle.output_id, now).is_ok());
    assert_eq!(store.live_count(), 0);
    // A second release says the output is gone rather than pretending.
    assert_eq!(
        store.release(&handle.output_id, now),
        Err(OutputError::UnknownOutput)
    );
}

#[test]
fn unknown_ids_are_refused_by_every_operation() {
    let store = OutputStore::new();
    let now = t0();
    assert_eq!(
        store.read_chunk("nope", 0, 1, now),
        Err(OutputError::UnknownOutput)
    );
    assert_eq!(store.release("nope", now), Err(OutputError::UnknownOutput));
}

#[test]
fn ids_are_not_recycled_after_a_release() {
    // A caller holding a stale id would otherwise read somebody else's document.
    let store = OutputStore::new();
    let now = t0();
    let first = store.insert(b"one".to_vec(), now).unwrap();
    store.release(&first.output_id, now).unwrap();
    let second = store.insert(b"two".to_vec(), now).unwrap();
    assert_ne!(first.output_id, second.output_id);
}

#[test]
fn an_empty_document_is_held_and_read_as_empty() {
    // `ExtractText` on a scanned PDF produces exactly this.
    let store = OutputStore::new();
    let now = t0();
    let handle = store.insert(Vec::new(), now).unwrap();
    assert_eq!(handle.total_bytes, 0);
    assert_eq!(handle.sha256, hex_digest(b""));
    assert_eq!(
        store.read_chunk(&handle.output_id, 0, 10, now).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn the_digest_matches_a_known_vector() {
    assert_eq!(
        hex_digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn debug_reports_sizes_not_contents() {
    // A derived Debug would put a whole document into a log line.
    let store = OutputStore::new();
    store
        .insert(b"secret contract text".to_vec(), t0())
        .unwrap();
    let rendered = format!("{store:?}");
    assert!(rendered.contains("live_outputs"));
    assert!(
        !rendered.contains("secret"),
        "document contents leaked into Debug: {rendered}"
    );
}
