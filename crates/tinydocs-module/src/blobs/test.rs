//! Unit tests for the chunked blob staging area.
//!
//! Weighted towards refusals on purpose. A happy-path transfer proves the store
//! can move bytes; it is the bounds and the expiry that decide whether an
//! abandoned upload leaks for the life of a module the host never unloads, and
//! whether a truncated transfer can be consumed as though it were whole.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{
    BlobError, BlobStore, IDLE_TTL, MAX_BLOB_BYTES, MAX_CHUNK_BYTES, MAX_LIVE_BLOBS,
    MAX_TOTAL_STAGED_BYTES, hex_digest, is_lowercase_sha256, verify,
};

/// A fixed origin for the injected clock, so every test is deterministic.
fn t0() -> Instant {
    Instant::now()
}

/// Stage `bytes` as a completed upload, the way a caller would.
fn upload(store: &BlobStore, bytes: &[u8], now: Instant) -> String {
    let id = store
        .begin(bytes.len() as u64, &hex_digest(bytes), now)
        .expect("begin should succeed");
    if !bytes.is_empty() {
        let received = store
            .put_chunk(&id, 0, bytes, now)
            .expect("put_chunk should succeed");
        assert_eq!(received, bytes.len() as u64);
    }
    id
}

#[test]
fn a_blob_round_trips_through_chunks() {
    let store = BlobStore::new();
    let now = t0();
    let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let digest = hex_digest(&payload);

    let id = store.begin(payload.len() as u64, &digest, now).unwrap();
    // Three uneven chunks, so the arithmetic is not accidentally aligned.
    let mut sent = 0usize;
    for size in [4_000usize, 4_000, 2_000] {
        let end = sent + size;
        let received = store
            .put_chunk(&id, sent as u64, &payload[sent..end], now)
            .unwrap();
        sent = end;
        assert_eq!(received, sent as u64);
    }

    let mut read = Vec::new();
    let mut offset = 0u64;
    while read.len() < payload.len() {
        let chunk = store.get_chunk(&id, offset, 3_000, now).unwrap();
        assert!(!chunk.is_empty(), "read stalled at offset {offset}");
        offset += chunk.len() as u64;
        read.extend_from_slice(&chunk);
    }
    assert_eq!(read, payload);
    assert_eq!(hex_digest(&read), digest);
}

#[test]
fn a_read_past_the_end_is_clamped_not_refused() {
    // So a caller can ask for a full chunk on the final read without having to
    // compute the remainder itself.
    let store = BlobStore::new();
    let now = t0();
    let id = upload(&store, b"twelve bytes", now);
    let chunk = store.get_chunk(&id, 6, 1_000_000, now).unwrap();
    assert_eq!(chunk, b" bytes");
}

#[test]
fn a_read_starting_past_the_end_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    let id = upload(&store, b"short", now);
    assert_eq!(
        store.get_chunk(&id, 99, 10, now),
        Err(BlobError::ReadPastEnd)
    );
}

#[test]
fn an_out_of_order_chunk_is_refused_and_names_both_offsets() {
    let store = BlobStore::new();
    let now = t0();
    let payload = vec![7u8; 100];
    let id = store.begin(100, &hex_digest(&payload), now).unwrap();
    store.put_chunk(&id, 0, &payload[..40], now).unwrap();

    // A duplicated chunk and a skipped chunk are the two ways a transfer goes
    // wrong; both must fail here rather than corrupt the blob silently.
    assert_eq!(
        store.put_chunk(&id, 0, &payload[..40], now),
        Err(BlobError::OutOfOrderChunk {
            expected: 40,
            actual: 0
        })
    );
    assert_eq!(
        store.put_chunk(&id, 60, &payload[60..], now),
        Err(BlobError::OutOfOrderChunk {
            expected: 40,
            actual: 60
        })
    );
}

#[test]
fn a_chunk_past_the_declared_size_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    let payload = vec![1u8; 10];
    let id = store.begin(10, &hex_digest(&payload), now).unwrap();
    let one_too_many = [1u8; 11];
    assert_eq!(
        store.put_chunk(&id, 0, &one_too_many, now),
        Err(BlobError::OverlongBlob)
    );
}

#[test]
fn an_oversize_chunk_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    let payload = vec![0u8; MAX_CHUNK_BYTES + 1];
    let id = store
        .begin(payload.len() as u64, &hex_digest(&payload), now)
        .unwrap();
    assert_eq!(
        store.put_chunk(&id, 0, &payload, now),
        Err(BlobError::ChunkTooLarge)
    );
}

#[test]
fn a_digest_mismatch_drops_the_blob() {
    // The blob must not remain readable, and must not remain charged against the
    // budget, after failing its integrity check.
    let store = BlobStore::new();
    let now = t0();
    let claimed = hex_digest(b"what the caller promised");
    let id = store.begin(5, &claimed, now).unwrap();

    assert_eq!(
        store.put_chunk(&id, 0, b"other", now),
        Err(BlobError::DigestMismatch)
    );
    assert_eq!(store.live_count(), 0, "mismatched blob was retained");
    assert_eq!(store.get_chunk(&id, 0, 5, now), Err(BlobError::UnknownBlob));
}

#[test]
fn an_incomplete_blob_cannot_be_read_or_taken() {
    let store = BlobStore::new();
    let now = t0();
    let payload = vec![3u8; 100];
    let id = store.begin(100, &hex_digest(&payload), now).unwrap();
    store.put_chunk(&id, 0, &payload[..50], now).unwrap();

    assert_eq!(
        store.get_chunk(&id, 0, 10, now),
        Err(BlobError::IncompleteBlob)
    );
    assert_eq!(
        store.take_complete(&id, now),
        Err(BlobError::IncompleteBlob)
    );
}

#[test]
fn a_malformed_digest_is_refused_before_anything_is_reserved() {
    let store = BlobStore::new();
    let now = t0();
    for bad in [
        "",
        "abc",
        &"A".repeat(64), // uppercase
        &"g".repeat(64), // not hex
        &"a".repeat(63), // too short
        &"a".repeat(65), // too long
    ] {
        assert_eq!(
            store.begin(10, bad, now),
            Err(BlobError::MalformedDigest),
            "accepted {bad:?}"
        );
    }
    assert_eq!(store.live_count(), 0);
}

#[test]
fn a_blob_over_the_per_blob_limit_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    assert_eq!(
        store.begin(MAX_BLOB_BYTES as u64 + 1, &hex_digest(b"anything"), now),
        Err(BlobError::BlobTooLarge)
    );
    // A declared size beyond usize on a 32-bit host lands in the same refusal.
    assert_eq!(
        store.begin(u64::MAX, &hex_digest(b"anything"), now),
        Err(BlobError::BlobTooLarge)
    );
}

#[test]
fn the_staging_budget_is_reserved_at_begin_not_on_arrival() {
    // Reserving up front is what lets an admitted transfer always finish. If the
    // budget only counted bytes received, two callers could each be admitted for
    // 96 MiB and then fight over the last 32.
    let store = BlobStore::new();
    let now = t0();
    let big = MAX_TOTAL_STAGED_BYTES / 2;
    let digest = hex_digest(b"never sent");

    store.begin(big as u64, &digest, now).unwrap();
    store.begin(big as u64, &digest, now).unwrap();
    // Nothing has actually been uploaded, yet the area is full.
    assert_eq!(store.begin(1, &digest, now), Err(BlobError::StagingFull));
}

#[test]
fn too_many_live_blobs_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    let digest = hex_digest(b"x");
    for _ in 0..MAX_LIVE_BLOBS {
        store.begin(1, &digest, now).unwrap();
    }
    assert_eq!(store.live_count(), MAX_LIVE_BLOBS);
    assert_eq!(store.begin(1, &digest, now), Err(BlobError::TooManyBlobs));
}

#[test]
fn an_untouched_blob_expires_and_frees_its_budget() {
    // The bound that matters most: a caller that dies mid-upload must not leak
    // its partial blob for the life of a module the host never unloads.
    let store = BlobStore::new();
    let now = t0();
    let payload = vec![9u8; 1_000];
    let id = store
        .begin(payload.len() as u64, &hex_digest(&payload), now)
        .unwrap();
    store.put_chunk(&id, 0, &payload[..500], now).unwrap();
    assert_eq!(store.live_count(), 1);

    // Still live right on the boundary.
    let at_ttl = now + IDLE_TTL;
    assert!(store.get_chunk(&id, 0, 1, at_ttl).is_err()); // incomplete, but alive
    assert_eq!(store.live_count(), 1);

    // Past it, the next operation sweeps it away.
    let past_ttl = now + IDLE_TTL + Duration::from_secs(1);
    assert_eq!(
        store.put_chunk(&id, 500, &payload[500..], past_ttl),
        Err(BlobError::UnknownBlob)
    );
    assert_eq!(store.live_count(), 0);
}

#[test]
fn activity_keeps_a_slow_transfer_alive() {
    // The flip side: a transfer that is slow but live must never be reaped.
    let store = BlobStore::new();
    let mut now = t0();
    let payload = vec![4u8; 400];
    let id = store.begin(400, &hex_digest(&payload), now).unwrap();

    for start in (0..400).step_by(100) {
        // Each chunk arrives just inside the window, well past the total elapsed
        // TTL — three of these sum to more than IDLE_TTL.
        now += IDLE_TTL.saturating_sub(Duration::from_secs(1));
        store
            .put_chunk(&id, start as u64, &payload[start..start + 100], now)
            .expect("a touched blob must not expire");
    }
    assert_eq!(store.get_chunk(&id, 0, 400, now).unwrap(), payload);
}

#[test]
fn releasing_frees_the_budget_and_is_reported_once() {
    let store = BlobStore::new();
    let now = t0();
    let id = upload(&store, b"payload", now);
    assert!(store.release(&id, now).is_ok());
    assert_eq!(store.live_count(), 0);
    // A second release tells the caller the blob is gone rather than pretending.
    assert_eq!(store.release(&id, now), Err(BlobError::UnknownBlob));
}

#[test]
fn taking_a_blob_removes_it() {
    let store = BlobStore::new();
    let now = t0();
    let id = upload(&store, b"consume me", now);
    assert_eq!(store.take_complete(&id, now).unwrap(), b"consume me");
    assert_eq!(store.live_count(), 0);
    assert_eq!(store.take_complete(&id, now), Err(BlobError::UnknownBlob));
}

#[test]
fn insert_complete_produces_a_readable_handle() {
    let store = BlobStore::new();
    let now = t0();
    let bytes = b"generated output".to_vec();
    let handle = store.insert_complete(bytes.clone(), now).unwrap();

    assert_eq!(handle.total_bytes, bytes.len() as u64);
    assert_eq!(handle.sha256, hex_digest(&bytes));
    assert_eq!(
        store.get_chunk(&handle.blob_id, 0, 1_000, now).unwrap(),
        bytes
    );
}

#[test]
fn insert_complete_respects_every_budget() {
    let store = BlobStore::new();
    let now = t0();
    assert_eq!(
        store.insert_complete(vec![0u8; MAX_BLOB_BYTES + 1], now),
        Err(BlobError::BlobTooLarge)
    );

    // Filling the staging area takes more than one blob: the per-blob cap is
    // half the total, so the area can only ever be filled by at least two.
    let full = BlobStore::new();
    let digest = hex_digest(b"reserved");
    let per_blob = MAX_BLOB_BYTES;
    let mut reserved = 0usize;
    while reserved + per_blob <= MAX_TOTAL_STAGED_BYTES {
        full.begin(per_blob as u64, &digest, now).unwrap();
        reserved += per_blob;
    }
    assert_eq!(reserved, MAX_TOTAL_STAGED_BYTES, "area not fully reserved");
    assert_eq!(
        full.insert_complete(vec![0u8; 16], now),
        Err(BlobError::StagingFull)
    );

    let crowded = BlobStore::new();
    for _ in 0..MAX_LIVE_BLOBS {
        crowded.begin(1, &digest, now).unwrap();
    }
    assert_eq!(
        crowded.insert_complete(vec![0u8; 1], now),
        Err(BlobError::TooManyBlobs)
    );
}

#[test]
fn a_zero_length_blob_completes_at_begin() {
    // There is no chunk to complete it on, so the digest has to be checked when
    // it is declared or the blob would never become readable.
    let store = BlobStore::new();
    let now = t0();
    let id = store.begin(0, &hex_digest(b""), now).unwrap();
    assert_eq!(store.get_chunk(&id, 0, 10, now).unwrap(), Vec::<u8>::new());
    assert_eq!(store.take_complete(&id, now).unwrap(), Vec::<u8>::new());
}

#[test]
fn a_zero_length_blob_with_a_wrong_digest_is_refused_at_begin() {
    let store = BlobStore::new();
    let now = t0();
    assert_eq!(
        store.begin(0, &hex_digest(b"not empty"), now),
        Err(BlobError::DigestMismatch)
    );
    assert_eq!(store.live_count(), 0);
}

#[test]
fn blob_ids_are_unique_across_reuse() {
    // Ids must not be recycled after a release: a caller holding a stale id
    // would otherwise read somebody else's blob.
    let store = BlobStore::new();
    let now = t0();
    let first = upload(&store, b"one", now);
    store.release(&first, now).unwrap();
    let second = upload(&store, b"two", now);
    assert_ne!(first, second);
}

#[test]
fn unknown_ids_are_refused_by_every_operation() {
    let store = BlobStore::new();
    let now = t0();
    assert_eq!(
        store.put_chunk("nope", 0, b"x", now),
        Err(BlobError::UnknownBlob)
    );
    assert_eq!(
        store.get_chunk("nope", 0, 1, now),
        Err(BlobError::UnknownBlob)
    );
    assert_eq!(
        store.take_complete("nope", now),
        Err(BlobError::UnknownBlob)
    );
    assert_eq!(store.release("nope", now), Err(BlobError::UnknownBlob));
}

#[test]
fn an_oversize_read_length_is_refused() {
    let store = BlobStore::new();
    let now = t0();
    let id = upload(&store, b"small", now);
    assert_eq!(
        store.get_chunk(&id, 0, MAX_CHUNK_BYTES as u64 + 1, now),
        Err(BlobError::ChunkTooLarge)
    );
    assert_eq!(
        store.get_chunk(&id, 0, u64::MAX, now),
        Err(BlobError::ChunkTooLarge)
    );
}

#[test]
fn digest_helpers_agree_with_a_known_vector() {
    // The SHA-256 of the empty string, so a wrong hasher or a broken hex
    // encoding fails here rather than in an integration test.
    assert_eq!(
        hex_digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert!(verify(b"", &hex_digest(b"")));
    assert!(!verify(b"", &hex_digest(b"x")));
    // A length mismatch must fail before the comparison loop.
    assert!(!verify(b"", "abcd"));
}

#[test]
fn digest_shape_is_validated_strictly() {
    assert!(is_lowercase_sha256(&hex_digest(b"anything")));
    assert!(!is_lowercase_sha256(
        &hex_digest(b"anything").to_uppercase()
    ));
    assert!(!is_lowercase_sha256("zz"));
}
