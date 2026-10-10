//! Markdown compatibility regressions using local PDF/Office fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use std::fmt::Write as _;

/// A deflated zip archive of `(path, contents)` parts.
fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;

    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, contents) in parts {
        writer.start_file(*path, options).unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    buffer.into_inner()
}

/// A Word document whose body is `paragraphs`, each a list of text runs.
fn docx(paragraphs: &[&[&str]]) -> Vec<u8> {
    let mut body = String::new();
    for runs in paragraphs {
        body.push_str("<w:p>");
        for run in *runs {
            write!(body, "<w:r><w:t>{run}</w:t></w:r>").unwrap();
        }
        body.push_str("</w:p>");
    }
    let document = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>{body}</w:body></w:document>"#
    );
    package(&[("word/document.xml", &document)])
}

/// A deck holding one slide per `(slide number, text)` pair, written to the
/// archive in the order given.
fn pptx(slides: &[(u32, &str)]) -> Vec<u8> {
    let mut parts = vec![(
        "ppt/presentation.xml".to_string(),
        r#"<?xml version="1.0"?><p:presentation xmlns:p="x"/>"#.to_string(),
    )];
    for (number, text) in slides {
        parts.push((
            format!("ppt/slides/slide{number}.xml"),
            format!(
                r#"<?xml version="1.0"?><p:sld xmlns:p="x" xmlns:a="y"><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>"#
            ),
        ));
    }
    let borrowed: Vec<(&str, &str)> = parts
        .iter()
        .map(|(path, contents)| (path.as_str(), contents.as_str()))
        .collect();
    package(&borrowed)
}

/// A minimal XLSX archive from a worksheet's `sheetData` fragment.
///
/// The parts are the smallest set calamine's Xlsx reader accepts: the
/// content-type map, the root and workbook relationships, and the workbook
/// itself. No shared strings or styles, which the reader tolerates.
fn xlsx_with_sheet(sheet_data: &str) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;
    let root_rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
    let workbook = r#"<?xml version="1.0"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let worksheet = format!(
        r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>{sheet_data}</sheetData>
</worksheet>"#
    );
    package(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", workbook_rels),
        ("xl/worksheets/sheet1.xml", &worksheet),
    ])
}

/// A one-page PDF whose content stream is `content`, with a correct
/// cross-reference table so the parser takes the normal path rather than a
/// recovery one.
fn pdf(content: &str) -> Vec<u8> {
    pdf_pages(&[content])
}

/// A PDF with one page per content stream, in order.
fn pdf_pages(contents: &[&str]) -> Vec<u8> {
    // 1 catalog, 2 pages, 3 font, then each page and its content stream.
    let kids: Vec<String> = (0..contents.len())
        .map(|index| format!("{} 0 R", 4 + 2 * index))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            contents.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (index, content) in contents.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + 2 * index
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        write!(out, "{} 0 obj\n{object}\nendobj\n", index + 1).unwrap();
    }
    let xref = out.len();
    write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(out, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    out.into_bytes()
}

fn convert(name: &str, bytes: &[u8]) -> Result<String> {
    let format = match name.rsplit('.').next().unwrap() {
        "pdf" => DocumentFormat::Pdf,
        "docx" => DocumentFormat::Docx,
        "pptx" => DocumentFormat::Pptx,
        "xlsx" => DocumentFormat::Xlsx,
        _ => panic!("test fixture must name an office format"),
    };
    super::convert(bytes, format)
}

fn refusal(name: &str, bytes: &[u8]) -> String {
    convert(name, bytes).unwrap_err().to_string()
}
#[test]
fn a_docx_yields_its_paragraphs_in_order() {
    let bytes = docx(&[&["First heading"], &["Second ", "paragraph."]]);
    let converted = convert("spec.docx", &bytes).unwrap();
    let text = &converted;
    assert!(text.starts_with("First heading"), "{text}");
    assert!(
        text.contains("Second paragraph."),
        "runs inside one paragraph join without a break: {text}"
    );
    assert!(
        text.contains("First heading\n\nSecond"),
        "paragraphs keep their break: {text:?}"
    );
}

#[test]
fn escaped_characters_in_a_run_survive_extraction() {
    // XML escapes `&` and `<` in text, and the parser reports each escape as
    // its own event: dropping those events would turn "Q&A" into "QA".
    let bytes = docx(&[&["Q&amp;A: 3 &lt; 4 &#38; &quot;done&quot;"]]);
    let converted = convert("faq.docx", &bytes).unwrap();
    assert_eq!(converted, "Q&A: 3 < 4 & \"done\"");
}

#[test]
fn a_deck_reads_its_slides_in_numeric_order() {
    // `slide10.xml` sorts before `slide2.xml` as a string, and a deck that
    // recalls out of order is worse than one that does not recall at all.
    let bytes = pptx(&[(10, "Tenth"), (2, "Second"), (1, "First")]);
    let converted = convert("deck.pptx", &bytes).unwrap();
    assert_eq!(converted, "First\n\nSecond\n\nTenth");
}

#[test]
fn a_small_spreadsheet_yields_one_line_per_row() {
    // The dense-range guard must not refuse ordinary files, and the concrete
    // (non-auto) Xlsx open must not either.
    let bytes = xlsx_with_sheet(
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>alpha</v></c></row>
           <row r="2"><c r="A2"><v>2</v></c><c r="B2"><v>beta</v></c></row>"#,
    );
    let converted = convert("ledger.xlsx", &bytes).unwrap();
    assert_eq!(converted, "Sheet1 | 1 | alpha\nSheet1 | 2 | beta");
}

#[test]
fn a_spreadsheet_with_far_cells_is_refused_not_allocated() {
    // One cell at `A1` and one at `XFD1048576` passes the decompression cap
    // (the archive is a few hundred bytes) yet would make calamine
    // materialize a ~17-billion-cell dense grid. Refused before that.
    let bytes = xlsx_with_sheet(
        r#"<row r="1"><c r="A1"><v>1</v></c></row>
           <row r="1048576"><c r="XFD1048576"><v>2</v></c></row>"#,
    );
    let reason = refusal("spread.xlsx", &bytes);
    assert!(reason.contains("used range"), "{reason}");
}

#[test]
fn an_overexpanding_document_is_refused_not_allocated() {
    // One entry of zero bytes just over the cap: zeroes compress to almost
    // nothing, so the archive is tiny while its declared expansion exceeds
    // the limit — exactly the shape of a crafted bomb.
    let zeroes = "\0".repeat(usize::try_from(MAX_DECOMPRESSED_BYTES).unwrap() + 1);
    let bytes = package(&[("word/document.xml", &zeroes)]);
    assert!(
        bytes.len() < MAX_DOCUMENT_BYTES,
        "the fixture must pass intake's own cap"
    );
    let reason = refusal("bomb.docx", &bytes);
    assert!(reason.contains("expands"), "{reason}");
}

#[test]
fn an_overexpanding_spreadsheet_is_refused_before_calamine_opens_it() {
    let zeroes = "\0".repeat(usize::try_from(MAX_DECOMPRESSED_BYTES).unwrap() + 1);
    let bytes = package(&[("xl/workbook.xml", &zeroes)]);
    let reason = refusal("bomb.xlsx", &bytes);
    assert!(reason.contains("expands"), "{reason}");
}

#[test]
fn a_pdf_yields_its_text_layer() {
    let bytes = pdf("BT /F1 24 Tf 72 720 Td (Quarterly revenue rose) Tj ET");
    let converted = convert("report.pdf", &bytes).unwrap();
    assert!(
        converted.contains("Quarterly revenue rose"),
        "{converted:?}"
    );
}

#[test]
fn a_pdf_keeps_its_page_boundaries_as_page_breaks() {
    let bytes = pdf_pages(&[
        "BT /F1 24 Tf 72 720 Td (Refunds take five days) Tj ET",
        "BT /F1 24 Tf 72 720 Td (Shipping is free over fifty) Tj ET",
        "",
        "BT /F1 24 Tf 72 720 Td (Returns need a receipt) Tj ET",
    ]);
    let converted = convert("policy.pdf", &bytes).unwrap();
    let pages: Vec<&str> = converted.split(PAGE_BREAK).collect();
    assert_eq!(pages.len(), 4, "{converted:?}");
    assert!(pages[0].contains("Refunds take five days"), "{pages:?}");
    assert!(
        pages[1].contains("Shipping is free over fifty"),
        "{pages:?}"
    );
    assert!(pages[2].trim().is_empty(), "an empty page keeps its place");
    assert!(pages[3].contains("Returns need a receipt"), "{pages:?}");
}

#[test]
fn a_pdf_without_a_text_layer_says_so_rather_than_storing_nothing() {
    // A scanned PDF carries pictures of words. That is not a parse failure,
    // but storing an empty body would lose the upload while looking like one
    // succeeded.
    let reason = refusal("scan.pdf", &pdf(""));
    assert!(reason.contains("no text"), "{reason}");
    // Several empty pages are joined by page breaks, which are not text.
    let reason = refusal("scan.pdf", &pdf_pages(&["", "", ""]));
    assert!(reason.contains("no text"), "{reason}");
}

#[test]
fn a_malformed_pdf_is_an_error_not_a_crash() {
    let reason = refusal("broken.pdf", b"%PDF-1.7\nthis is not a pdf");
    assert!(reason.contains("PDF"), "{reason}");
}

#[test]
fn a_document_with_no_text_is_an_error_not_an_empty_document() {
    let reason = refusal("blank.docx", &docx(&[&[" "]]));
    assert!(reason.contains("produced no text"), "{reason}");
}

#[test]
fn input_limits_are_checked_before_the_parser_runs() {
    for bytes in [Vec::new(), vec![0; MAX_DOCUMENT_BYTES + 1]] {
        assert!(matches!(
            super::convert(&bytes, DocumentFormat::Pdf),
            Err(Error::InvalidInput { .. })
        ));
    }
}

#[test]
fn malformed_office_archives_are_extraction_errors() {
    for format in [
        DocumentFormat::Docx,
        DocumentFormat::Pptx,
        DocumentFormat::Xlsx,
    ] {
        assert!(matches!(
            super::convert(b"plain words", format),
            Err(Error::ExtractionFailed { .. })
        ));
    }
}

#[test]
fn normalization_collapses_whitespace_without_merging_paragraphs() {
    assert_eq!(
        normalize::normalize("  First   line  \n\n\n  second\tline \n"),
        "First line\n\n second line"
    );
}

#[test]
fn a_pdf_with_an_invalid_text_operand_is_refused_even_if_the_parser_panics() {
    let bytes = pdf("BT /F1 24 Tf 72 720 Td 42 Tj ET");
    let reason = refusal("malformed-operand.pdf", &bytes);
    assert!(reason.contains("parser gave up"), "{reason}");
}

#[test]
fn spreadsheet_blank_rows_and_cells_preserve_the_remaining_row_labels() {
    let bytes = xlsx_with_sheet(
        r#"<row r="1"><c r="A1"><v>1</v></c></row>
        <row r="3"><c r="B3"><v>3</v></c></row>"#,
    );
    assert_eq!(
        convert("gaps.xlsx", &bytes).unwrap(),
        "Sheet1 | 1 |\nSheet1 | | 3"
    );
    let empty = xlsx_with_sheet("");
    assert!(refusal("empty.xlsx", &empty).contains("no text"));
}

#[test]
fn a_readable_archive_without_a_workbook_is_an_extraction_error() {
    let bytes = package(&[("unrelated.xml", "<document/>")]);
    assert!(refusal("not-a-workbook.xlsx", &bytes).contains("could not be read"));
}
