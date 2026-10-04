//! Hostile metadata is rejected before the eager ZIP index is constructed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::intake::{DocumentFormat, ExtractDocumentSpec, extract};
use std::io::{Cursor, Write};

fn fixture(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn docx() -> Vec<u8> {
    fixture(&[("word/document.xml", b"<t>visible</t>")])
}
fn extract_docx(bytes: &[u8]) -> crate::Result<crate::intake::ExtractedDocument> {
    extract(bytes, &ExtractDocumentSpec::new(DocumentFormat::Docx))
}
fn zip64(bytes: &[u8]) -> Vec<u8> {
    let end = bytes.len() - 22;
    let mut result = bytes[..end].to_vec();
    result.extend_from_slice(b"PK\x06\x06");
    result.extend_from_slice(&44u64.to_le_bytes());
    result.extend_from_slice(&[0; 12]);
    result.extend_from_slice(&1u64.to_le_bytes());
    result.extend_from_slice(&1u64.to_le_bytes());
    result.extend_from_slice(&number(bytes, end + 12, 4).unwrap().to_le_bytes());
    result.extend_from_slice(&number(bytes, end + 16, 4).unwrap().to_le_bytes());
    result.extend_from_slice(b"PK\x06\x07");
    result.extend_from_slice(&0u32.to_le_bytes());
    result.extend_from_slice(&(end as u64).to_le_bytes());
    result.extend_from_slice(&1u32.to_le_bytes());
    result.extend_from_slice(&bytes[end..]);
    let footer = result.len() - 22;
    result[footer + 8..footer + 12].fill(255);
    result[footer + 12..footer + 20].fill(255);
    result
}

#[test]
fn admits_valid_zip64_and_rejects_hostile_declared_counts() {
    let bytes = docx();
    let valid = zip64(&bytes);
    assert_eq!(extract_docx(&valid).unwrap().sections[0].text, "visible");
    let end = bytes.len() - 22;
    let mut hostile = bytes.clone();
    hostile[end + 8..end + 12].fill(255);
    assert!(
        extract_docx(&hostile)
            .unwrap_err()
            .to_string()
            .contains("invalid ZIP central")
    );
    let mut hostile64 = valid.clone();
    hostile64[end + 24..end + 40].fill(255);
    assert!(
        extract_docx(&hostile64)
            .unwrap_err()
            .to_string()
            .contains("invalid ZIP central")
    );
    let mut disagreement = valid;
    let footer = disagreement.len() - 22;
    disagreement[footer + 10..footer + 12].copy_from_slice(&2u16.to_le_bytes());
    assert!(zip_preflight(&disagreement).is_err());
}

#[test]
fn rejects_split_archives_impossible_offsets_and_malformed_zip64() {
    let bytes = docx();
    let end = bytes.len() - 22;
    for (position, value) in [
        (end + 4, 1),
        (end + 6, 1),
        (end + 8, 2),
        (end + 12, 255),
        (end + 16, 255),
    ] {
        let mut malformed = bytes.clone();
        malformed[position] = value;
        assert!(zip_preflight(&malformed).is_err());
    }
    let valid = zip64(&bytes);
    for position in [
        end,
        end + 4,
        end + 16,
        end + 20,
        end + 56 + 4,
        end + 56 + 8,
        end + 56 + 16,
    ] {
        let mut malformed = valid.clone();
        malformed[position] = 255;
        assert!(zip_preflight(&malformed).is_err(), "offset {position}");
    }
    for bytes in [b"".as_slice(), b"PK\x05\x06", b"not a zip"] {
        assert!(zip_preflight(bytes).is_err());
    }
}

#[test]
fn bounds_unicode_path_extra_fields_before_allocating_replacement_names() {
    let bytes = docx();
    let end = bytes.len() - 22;
    let size = usize::try_from(number(&bytes, end + 12, 4).unwrap()).unwrap();
    let central = end - size;
    let name_len = usize::try_from(number(&bytes, central + 28, 2).unwrap()).unwrap();
    let mut extra = Vec::new();
    extra.extend_from_slice(&0x7075u16.to_le_bytes());
    extra.extend_from_slice(&262u16.to_le_bytes());
    extra.extend_from_slice(&[1, 0, 0, 0, 0]);
    extra.extend_from_slice(&[b'x'; 257]);
    let mut hostile = bytes.clone();
    hostile.splice(
        central + 46 + name_len..central + 46 + name_len,
        extra.iter().copied(),
    );
    hostile[central + 30..central + 32]
        .copy_from_slice(&u16::try_from(extra.len()).unwrap().to_le_bytes());
    let new_footer = hostile.len() - 22;
    hostile[new_footer + 12..new_footer + 16]
        .copy_from_slice(&u32::try_from(size + extra.len()).unwrap().to_le_bytes());
    assert!(
        extract_docx(&hostile)
            .unwrap_err()
            .to_string()
            .contains("ZIP admission name limit")
    );
    let mut malformed = hostile.clone();
    malformed[central + 46 + name_len + 2..central + 46 + name_len + 4].fill(255);
    assert!(zip_preflight(&malformed).is_err());
}

#[test]
fn central_directory_metadata_and_zip64_extensions_have_fixed_caps() {
    let count = 17u16;
    let mut bytes = Vec::new();
    for _ in 0..count {
        let mut header = [0u8; 46];
        header[..4].copy_from_slice(b"PK\x01\x02");
        header[28..30].copy_from_slice(&1u16.to_le_bytes());
        header[30..32].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.push(b'x');
        bytes.extend_from_slice(&vec![0; usize::from(u16::MAX)]);
    }
    let size = u32::try_from(bytes.len()).unwrap();
    let mut footer = [0u8; 22];
    footer[..4].copy_from_slice(b"PK\x05\x06");
    footer[8..10].copy_from_slice(&count.to_le_bytes());
    footer[10..12].copy_from_slice(&count.to_le_bytes());
    footer[12..16].copy_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&footer);
    assert!(
        extract_docx(&bytes)
            .unwrap_err()
            .to_string()
            .contains("ZIP admission metadata limit")
    );

    let original = docx();
    let offset = original.len() - 22;
    let mut huge = zip64(&original);
    let extra = 1024 * 1024;
    huge.splice(offset + 56..offset + 56, std::iter::repeat_n(0, extra));
    huge[offset + 4..offset + 12].copy_from_slice(&(44 + extra as u64).to_le_bytes());
    assert!(
        extract_docx(&huge)
            .unwrap_err()
            .to_string()
            .contains("ZIP admission metadata limit")
    );
}

#[test]
fn payload_footer_cannot_be_retried_when_primary_directory_is_invalid() {
    let nested = fixture(&[("nested.xml", b"<t>unchecked</t>")]);
    let bytes = fixture(&[
        ("nested.zip", &nested),
        ("word/document.xml", b"<t>visible</t>"),
    ]);
    assert_eq!(extract_docx(&bytes).unwrap().sections[0].text, "visible");
    let mut prefixed = b"preamble".to_vec();
    prefixed.extend_from_slice(&bytes);
    assert_eq!(extract_docx(&prefixed).unwrap().sections[0].text, "visible");
    let end = bytes.len() - 22;
    let size = usize::try_from(number(&bytes, end + 12, 4).unwrap()).unwrap();
    let central = end - size;
    let mut corrupt = bytes.clone();
    // AES without its required metadata makes the eager parser search earlier
    // footers; the embedded ZIP must not become a second admission candidate.
    corrupt[central + 10..central + 12].copy_from_slice(&99u16.to_le_bytes());
    zip_preflight(&corrupt).unwrap();
    assert!(extract_docx(&corrupt).is_err());
}

#[test]
fn alternate_footer_signatures_in_metadata_fail_closed() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.set_comment("comment PK\u{5}\u{6} footer").unwrap();
    writer
        .start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"<t>visible</t>").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(extract_docx(&bytes).is_err());
}
