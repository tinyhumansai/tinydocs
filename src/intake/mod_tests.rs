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
    for (prefix, manifest, rels, root, container, element, kind) in [
        (
            "ppt/slides/slide",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "presentation",
            "sldIdLst",
            "sldId",
            "slide",
        ),
        (
            "xl/worksheets/sheet",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "workbook",
            "sheets",
            "sheet",
            "worksheet",
        ),
    ] {
        let mut numbered: Vec<_> = parts
            .iter()
            .filter_map(|(name, _)| {
                name.strip_prefix(prefix)
                    .and_then(|value| value.strip_suffix(".xml"))
                    .and_then(|value| value.parse::<u32>().ok())
                    .map(|n| (n, *name))
            })
            .collect();
        numbered.sort_unstable();
        if numbered.is_empty() || parts.iter().any(|(name, _)| *name == manifest) {
            continue;
        }
        let mut items = String::new();
        for (n, _) in &numbered {
            use std::fmt::Write as _;
            write!(&mut items, "<{element} r:id='r{n}'/>").unwrap();
        }
        let xml = format!("<{root} xmlns:r='r'><{container}>{items}</{container}></{root}>");
        zip.start_file(manifest, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let mut relationships = String::new();
        for (n, name) in &numbered {
            use std::fmt::Write as _;
            write!(&mut relationships, "<Relationship Id='r{n}' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}' Target='{}'/>", name.split_once('/').unwrap().1).unwrap();
        }
        zip.start_file(rels, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(format!("<Relationships>{relationships}</Relationships>").as_bytes())
            .unwrap();
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
fn extracts_slides_in_manifest_order_and_excludes_metadata() {
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

#[test]
fn zip_member_limits_are_rejected_by_admission_before_eager_indexing() {
    let names: Vec<_> = (0..=MAX_ENTRIES).map(|i| format!("part{i}")).collect();
    let parts: Vec<_> = names.iter().map(|name| (name.as_str(), "")).collect();
    let error = extract(
        &archive(&parts),
        &ExtractDocumentSpec::new(DocumentFormat::Docx),
    )
    .unwrap_err();
    assert!(error.to_string().contains("ZIP admission count limit"));
    let error = extract(
        &archive(&[(&"x".repeat(257), "")]),
        &ExtractDocumentSpec::new(DocumentFormat::Docx),
    )
    .unwrap_err();
    assert!(error.to_string().contains("ZIP admission name limit"));
}

#[test]
fn worksheet_expansion_is_bounded_while_resolving_repeated_shared_strings() {
    let shared = vec!["é".repeat(50_000)];
    let xml = format!(
        "<worksheet>{}</worksheet>",
        "<c t='s'><v>0</v></c>".repeat(100)
    );
    let text = xml_text(xml.as_bytes(), &shared, 31).unwrap();
    assert!(
        text.text.len() <= 31,
        "shared strings must be bounded before appending"
    );
    assert!(text.truncated);
    assert!(text.text.capacity() <= 31);
    let strings = format!("<sst><si><t>{}</t></si></sst>", shared[0]);
    let bytes = archive(&[
        ("xl/sharedStrings.xml", &strings),
        ("xl/worksheets/sheet1.xml", &xml),
    ]);
    let mut spec = ExtractDocumentSpec::new(DocumentFormat::Xlsx);
    spec.max_text_bytes = 31;
    let result = extract(&bytes, &spec).unwrap();
    assert!(result.truncated);
    assert_eq!(result.section_count, 1);
    assert_eq!(result.sections[0].text, "é".repeat(15));
}

#[test]
fn office_manifest_order_excludes_orphans_and_preserves_part_provenance() {
    let bytes = archive(&[
        (
            "ppt/presentation.xml",
            "<p:presentation xmlns:p='p' xmlns:r='r'><p:sldIdLst><p:sldId r:id='r2'/><p:sldId r:id='r1'/></p:sldIdLst></p:presentation>",
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            "<Relationships><Relationship Id='r1' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide' Target='slides/slide1.xml'/><Relationship Id='r2' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide' Target='slides/slide2.xml'/></Relationships>",
        ),
        ("ppt/slides/slide1.xml", "<t>first filename</t>"),
        ("ppt/slides/slide2.xml", "<t>first displayed</t>"),
        ("ppt/slides/slide3.xml", "<t>orphan</t>"),
    ]);
    let result = extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Pptx)).unwrap();
    assert_eq!(result.section_count, 2);
    assert_eq!(result.sections[0].source, "ppt/slides/slide2.xml");
    assert_eq!(result.sections[0].text, "first displayed");
    assert_eq!(result.sections[1].source, "ppt/slides/slide1.xml");
}

#[test]
fn workbook_order_uses_relationships_instead_of_names_and_ignores_orphans() {
    let bytes = archive(&[
        (
            "xl/workbook.xml",
            "<workbook xmlns:r='r'><sheets><sheet name='Visible B' r:id='b'/><sheet name='Visible A' r:id='a'/></sheets></workbook>",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id='a' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet' Target='worksheets/alpha.xml'></Relationship><Relationship Id='b' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet' Target='/xl/worksheets/beta.xml'/></Relationships>",
        ),
        ("xl/worksheets/alpha.xml", "<worksheet><t>A</t></worksheet>"),
        ("xl/worksheets/beta.xml", "<worksheet><t>B</t></worksheet>"),
        (
            "xl/worksheets/sheet3.xml",
            "<worksheet><t>orphan</t></worksheet>",
        ),
    ]);
    let mut spec = ExtractDocumentSpec::new(DocumentFormat::Xlsx);
    spec.max_sections = 1;
    let result = extract(&bytes, &spec).unwrap();
    assert_eq!(result.section_count, 2);
    assert!(result.truncated);
    assert_eq!(result.sections[0].source, "xl/worksheets/beta.xml");
    assert_eq!(result.sections[0].index, 1);
    assert_eq!(result.sections[0].text, "B");
}

#[test]
fn invalid_document_relationships_and_manifests_fail_explicitly() {
    let manifest = "<workbook xmlns:r='r'><sheets><sheet r:id='a'/></sheets></workbook>";
    for relationships in [
        "<Relationships/>",
        "<Relationships><Relationship Id='a' Type='x/worksheet' Target='https://example.com/sheet.xml' TargetMode='External'/></Relationships>",
        "<Relationships><Relationship Id='a' Type='x/worksheet' Target='../../outside.xml'/></Relationships>",
        "<Relationships><Relationship Id='a' Type='x/worksheet' Target='worksheets/missing.xml'/></Relationships>",
        "<Relationships><Relationship Id='a' Type='x/worksheet' Target='worksheets/sheet1.xml'/><Relationship Id='a' Type='x/worksheet' Target='worksheets/sheet1.xml'/></Relationships>",
        "<Relationships><Relationship Id='a' Type='x/not-worksheet' Target='worksheets/sheet1.xml'/></Relationships>",
        "<!DOCTYPE Relationships><Relationships/>",
    ] {
        let bytes = archive(&[
            ("xl/workbook.xml", manifest),
            ("xl/_rels/workbook.xml.rels", relationships),
            ("xl/worksheets/sheet1.xml", "<worksheet/>"),
        ]);
        assert!(extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Xlsx)).is_err());
    }
    for manifest in [
        "<workbook><sheets><sheet/></sheets></workbook>",
        "<workbook xmlns:r='r'><sheets><sheet r:id='a'/><sheet r:id='a'/></sheets></workbook>",
        "<workbook><sheets>",
    ] {
        let bytes = archive(&[
            ("xl/workbook.xml", manifest),
            (
                "xl/_rels/workbook.xml.rels",
                "<Relationships><Relationship Id='a' Type='x/worksheet' Target='worksheets/sheet1.xml'/></Relationships>",
            ),
            ("xl/worksheets/sheet1.xml", "<worksheet/>"),
        ]);
        assert!(extract(&bytes, &ExtractDocumentSpec::new(DocumentFormat::Xlsx)).is_err());
    }
}

#[test]
fn budget_exhaustion_still_validates_xml_and_shared_string_indices() {
    let shared = vec!["large output".repeat(1000)];
    for xml in [
        "<worksheet><c t='s'><v>0</v></c><t>&undefined;</t></worksheet>",
        "<worksheet><c t='s'><v>0</v></c><c t='s'><v>999999999999999999999</v></c></worksheet>",
        "<worksheet><c t='s'><v>0</v></c><c t='s'><v>42</v></c></worksheet>",
        "<worksheet><c t='s'><v>0</v></c>",
    ] {
        assert!(xml_text(xml.as_bytes(), &shared, 1).is_err(), "{xml}");
    }
    let output = xml_text(b"<t>abcdef&amp;<![CDATA[more]]></t>", &[], 3).unwrap();
    assert_eq!(output.text, "abc");
    assert!(output.truncated);
    assert_eq!(output.text.capacity(), 3);
}

#[test]
fn shared_strings_enforce_count_and_allocation_budgets_before_push() {
    let too_many = format!("<sst>{}</sst>", "<si/>".repeat(65_537));
    assert!(xml_strings(too_many.as_bytes()).is_err());
    let too_large = format!(
        "<sst><si><t>{}</t></si></sst>",
        "a".repeat(4 * 1024 * 1024 + 1)
    );
    assert!(xml_strings(too_large.as_bytes()).is_err());
    let accepted = xml_strings(b"<sst><si/><si><t>ok</t></si></sst>").unwrap();
    assert_eq!(accepted, vec!["", "ok"]);
}

#[test]
fn short_section_does_not_retain_unused_output_budget() {
    for xml in [b"<t/>".as_slice(), b"<t>ok</t>".as_slice()] {
        let text = xml_text(xml, &[], 200_000).unwrap();
        assert_eq!(text.text.capacity(), text.text.len());
    }
}

#[test]
fn explicit_office_breaks_and_tabs_preserve_word_boundaries() {
    let docx =
        b"<w:p><w:r><w:t>Hello</w:t><w:br/><w:t>world</w:t><w:tab/><w:t>end</w:t></w:r></w:p>";
    assert_eq!(
        xml_text(docx, &[], 1024).unwrap().text,
        "Hello\nworld\tend\n"
    );
    let pptx =
        b"<a:p><a:r><a:t>Hello</a:t></a:r><a:br><a:rPr/></a:br><a:r><a:t>world</a:t></a:r></a:p>";
    assert_eq!(xml_text(pptx, &[], 1024).unwrap().text, "Hello\nworld\n");
}

#[test]
fn worksheet_phonetic_annotations_do_not_change_shared_or_inline_values() {
    let shared = xml_strings("<sst><si><t>東京</t><rPh sb=\"0\" eb=\"2\"><t>とうきょう</t></rPh><r><t>駅</t></r></si></sst>".as_bytes()).unwrap();
    assert_eq!(shared, vec!["東京駅"]);
    let xml = "<row><c t=\"s\"><v>0</v></c><c t=\"inlineStr\"><is><t>東京</t><rPh><t>とうきょう</t></rPh><r><t>駅</t></r></is></c></row>";
    assert_eq!(
        xml_text(xml.as_bytes(), &shared, 1024).unwrap().text,
        "東京駅\n東京駅\n\n"
    );
}
