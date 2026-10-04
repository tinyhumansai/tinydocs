//! Bounded OOXML intake behavior.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::io::Write;
fn archive(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
#[test]
fn extracts_docx_visible_text_and_provenance() {
    let bytes = archive(&[(
        "word/document.xml",
        "<w:document xmlns:w='w'><w:p><w:r><w:t>Hello &amp; world</w:t></w:r></w:p></w:document>",
    )]);
    let result = extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Docx)).unwrap();
    assert_eq!(result.sections[0].source, "word/document.xml");
    assert!(result.sections[0].text.contains("Hello & world"));
}
#[test]
fn truncates_text_without_splitting_unicode() {
    let bytes = archive(&[("word/document.xml", "<w:t xmlns:w='w'>ééé</w:t>")]);
    let mut spec = ExtractDocumentSpec::new(DocumentFormat::Docx);
    spec.max_text_bytes = 3;
    let result = extract(&bytes, &spec).unwrap();
    assert!(result.truncated);
    assert_eq!(result.sections[0].text, "é");
}

#[test]
fn extracts_slides_in_numeric_order_and_excludes_metadata() {
    let bytes = archive(&[
        ("ppt/slides/slide10.xml", "<a:t xmlns:a='a'>ten</a:t>"),
        ("ppt/slides/slide2.xml", "<a:t xmlns:a='a'>two</a:t>"),
        (
            "docProps/core.xml",
            "<a:t xmlns:a='a'>private metadata</a:t>",
        ),
    ]);
    let result = extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Pptx)).unwrap();
    assert_eq!(result.section_count, 2);
    assert_eq!(result.sections[0].text, "two");
    assert_eq!(result.sections[1].text, "ten");
}
#[test]
fn resolves_xlsx_shared_inline_and_numeric_cell_values() {
    let bytes = archive(&[
        (
            "xl/sharedStrings.xml",
            "<sst><si><t>Hello &amp; </t><r><t>world</t></r></si></sst>",
        ),
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet><row><c t='s'><v>0</v></c><c t='inlineStr'><is><t>Inline</t></is></c><c><v>42</v></c></row></worksheet>",
        ),
    ]);
    let result = extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Xlsx)).unwrap();
    assert!(result.sections[0].text.contains("Hello & world"));
    assert!(result.sections[0].text.contains("Inline"));
    assert!(result.sections[0].text.contains("42"));
}
#[test]
fn preserves_text_scanned_and_mixed_pdf_page_provenance() {
    let bytes =
        crate::pdf::fixtures::document(&[("text", false), ("", true), ("mixed", true)], false);
    let result = extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Pdf)).unwrap();
    assert_eq!(result.section_count, 3);
    assert!(result.sections[0].text.contains("text"));
    assert!(result.sections[1].scanned_candidate);
    assert!(!result.sections[2].scanned_candidate);
    assert_eq!(result.sections[2].source, "page:3");
}
#[test]
fn pdf_text_and_section_budgets_are_explicit() {
    let bytes = crate::pdf::fixtures::document(&[("abcdef", false), ("second", false)], false);
    let mut spec = ExtractDocumentSpec::new(DocumentFormat::Pdf);
    spec.max_sections = 1;
    spec.max_text_bytes = 3;
    let result = extract(&bytes, &spec).unwrap();
    assert!(result.truncated);
    assert_eq!(result.section_count, 2);
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0].text.len() <= 3);
}
#[test]
fn rejects_bad_documents_budgets_and_xml_entities() {
    let mut spec = ExtractDocumentSpec::new(DocumentFormat::Docx);
    assert!(extract(b"not zip", &spec).is_err());
    for xml in [
        "<!DOCTYPE x [<!ENTITY a 'oops'>]><w:t>&a;</w:t>",
        "<w:t>unclosed",
        "<w:t>&unknown;</w:t>",
    ] {
        assert!(extract(&archive(&[("word/document.xml", xml)]), &spec).is_err());
    }
    let deep = "<p>".repeat(129) + &"</p>".repeat(129);
    assert!(extract(&archive(&[("word/document.xml", &deep)]), &spec).is_err());
    spec.max_text_bytes = 200_001;
    assert!(extract(b"zip", &spec).is_err());
    assert!(
        extract(
            &archive(&[("other.xml", "<t>ignore</t>")]),
            &ExtractDocumentSpec::new(DocumentFormat::Docx)
        )
        .is_err()
    );
}
#[test]
fn rejects_zip_expansion_and_member_count_limits() {
    let expanded = "a".repeat(usize::try_from(MAX_PART).unwrap() + 1);
    assert!(
        extract(
            &archive(&[("word/document.xml", &expanded)]),
            &ExtractDocumentSpec::new(DocumentFormat::Docx)
        )
        .is_err()
    );
    let names: Vec<String> = (0..=MAX_ENTRIES)
        .map(|index| format!("part{index}"))
        .collect();
    let parts: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "")).collect();
    assert!(
        extract(
            &archive(&parts),
            &ExtractDocumentSpec::new(DocumentFormat::Docx)
        )
        .is_err()
    );
}
#[test]
fn rejects_encrypted_and_malformed_pdf() {
    let spec = ExtractDocumentSpec::new(DocumentFormat::Pdf);
    assert!(
        extract(
            &crate::pdf::fixtures::document(&[("secret", false)], true),
            &spec
        )
        .is_err()
    );
    assert!(extract(b"%PDF-broken", &spec).is_err());
    assert!(extract(b"not pdf", &spec).is_err());
}

#[test]
fn preserves_cdata_empty_shared_strings_and_missing_shared_table() {
    let spec = ExtractDocumentSpec::new(DocumentFormat::Xlsx);
    let bytes = archive(&[
        (
            "xl/sharedStrings.xml",
            "<sst><si/><si><t><![CDATA[a < b]]></t></si></sst>",
        ),
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet><row><c t='s'><v>0</v></c><c t='s'><v>1</v></c></row></worksheet>",
        ),
    ]);
    let result = extract(&bytes, &spec).unwrap();
    assert!(result.sections[0].text.contains("a < b"));
    let inline = archive(&[(
        "xl/worksheets/sheet1.xml",
        "<worksheet><c t='inlineStr'><is><t><![CDATA[raw < text]]></t></is></c></worksheet>",
    )]);
    assert!(
        extract(&inline, &spec).unwrap().sections[0]
            .text
            .contains("raw < text")
    );
    let invalid_index = archive(&[(
        "xl/worksheets/sheet1.xml",
        "<worksheet><c t='s'><v>42</v></c></worksheet>",
    )]);
    assert!(extract(&invalid_index, &spec).is_err());
}
#[test]
fn rejects_long_member_names_empty_inputs_and_actual_expansion() {
    let spec = ExtractDocumentSpec::new(DocumentFormat::Docx);
    assert!(extract(&[], &spec).is_err());
    assert!(extract(&archive(&[(&"x".repeat(257), "")]), &spec).is_err());
    let mut expanded = MAX_EXPANDED;
    assert!(read_part(std::io::Cursor::new(b"x"), &mut expanded).is_err());
    let mut expanded = 0;
    assert!(read_part(std::io::repeat(b'x').take(MAX_PART + 1), &mut expanded).is_err());
}
#[test]
fn rejects_malformed_shared_string_xml() {
    let spec = ExtractDocumentSpec::new(DocumentFormat::Xlsx);
    for xml in [
        "<!DOCTYPE sst><sst/>",
        "<sst><si><t>unclosed",
        "<sst><si><t>&undefined;</t></si></sst>",
    ] {
        let bytes = archive(&[
            ("xl/sharedStrings.xml", xml),
            ("xl/worksheets/sheet1.xml", "<worksheet/>"),
        ]);
        assert!(extract(&bytes, &spec).is_err());
    }
    let deep = "<sst>".repeat(129) + &"</sst>".repeat(129);
    assert!(
        extract(
            &archive(&[
                ("xl/sharedStrings.xml", &deep),
                ("xl/worksheets/sheet1.xml", "<worksheet/>")
            ]),
            &spec
        )
        .is_err()
    );
}
