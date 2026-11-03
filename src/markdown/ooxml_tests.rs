//! Tests for Markdown extraction from OOXML document archives.

use super::{invalid_member_name, pptx, xml_text};
use std::io::Write;
use zip::unstable::write::FileOptionsExt;

fn deck_with_encrypted_unrelated_part() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive.start_file(
        "ppt/notesSlides/notesSlide1.xml",
        zip::write::SimpleFileOptions::default().with_deprecated_encryption(b"password")?,
    )?;
    archive.write_all(b"encrypted notes")?;
    archive.start_file(
        "ppt/slides/slide1.xml",
        zip::write::SimpleFileOptions::default(),
    )?;
    archive.write_all(b"<a:p><a:t>visible slide</a:t></a:p>")?;
    Ok(archive.finish()?.into_inner())
}

#[test]
fn unknown_entities_do_not_add_text() {
    assert_eq!(
        xml_text("<a:p><a:t>before&unknown;after</a:t></a:p>", "a:p", "a:t"),
        "beforeafter\n\n"
    );
}

#[test]
fn malformed_xml_keeps_text_read_before_the_error() {
    assert_eq!(
        xml_text("<a:p><a:t>before</a:t><broken", "a:p", "a:t"),
        "before"
    );
}

#[test]
fn invalid_member_name_errors_keep_archive_context() {
    let error = invalid_member_name(zip::result::ZipError::FileNotFound);
    assert!(
        error
            .to_string()
            .contains("deck contains an invalid member name")
    );
}

#[test]
fn encrypted_unrelated_parts_do_not_hide_valid_slides() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        pptx(&deck_with_encrypted_unrelated_part()?)?,
        "visible slide\n\n"
    );
    Ok(())
}
