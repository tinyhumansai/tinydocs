//! Bounded document text intake without filesystem or network access.
//!
//! OOXML reads selected ZIP members and parses XML events. PDF parsing is
//! CPU-bound; hosts own execution deadlines and any stronger isolation.
use crate::{Error, Result};
use quick_xml::{Reader, events::Event};
use std::io::{Cursor, Read};
pub use tinydocs_bus::intake::{
    DocumentFormat, DocumentSectionText, ExtractDocumentSpec, ExtractedDocument,
};

const MAX_INPUT: usize = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 32 * 1024 * 1024;
const MAX_PART: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;

/// Extract bounded visible text with document/page/slide/worksheet provenance.
///
/// # Errors
/// Rejects invalid budgets, malformed/encrypted documents, XML entities/DTDs,
/// excessive ZIP expansion, excessive XML nesting and unsupported structures.
pub fn extract(bytes: &[u8], spec: &ExtractDocumentSpec) -> Result<ExtractedDocument> {
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return Err(Error::invalid_input(
            "bytes",
            "document is empty or exceeds 64 MiB",
        ));
    }
    if spec.max_text_bytes == 0
        || spec.max_text_bytes > 200_000
        || spec.max_sections == 0
        || spec.max_sections > 256
    {
        return Err(Error::invalid_input(
            "spec",
            "text/section budget outside hard bounds",
        ));
    }
    if spec.format == DocumentFormat::Pdf {
        return extract_pdf(bytes, spec);
    }
    extract_office(bytes, spec)
}
fn extract_office(bytes: &[u8], spec: &ExtractDocumentSpec) -> Result<ExtractedDocument> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(failed)?;
    let parts = office_parts(&mut archive, spec.format)?;
    let mut expanded_read = 0u64;
    let shared = if spec.format == DocumentFormat::Xlsx {
        match archive.by_name("xl/sharedStrings.xml") {
            Ok(part) => xml_strings(&read_part(part, &mut expanded_read)?)?,
            Err(zip::result::ZipError::FileNotFound) => Vec::new(),
            Err(error) => return Err(failed(error)),
        }
    } else {
        Vec::new()
    };
    let mut result = ExtractedDocument {
        format: spec.format,
        section_count: u32::try_from(parts.len()).map_err(failed)?,
        sections: Vec::new(),
        truncated: parts.len() > spec.max_sections as usize,
    };
    let mut remaining = spec.max_text_bytes as usize;
    for (index, (name, part_index)) in parts
        .into_iter()
        .take(spec.max_sections as usize)
        .enumerate()
    {
        let xml = read_part(
            archive.by_index(part_index).map_err(failed)?,
            &mut expanded_read,
        )?;
        let text = xml_text(&xml, &shared)?;
        let bounded = bound_text(&text, remaining);
        remaining -= bounded.len();
        result.truncated |= bounded.len() < text.len();
        result.sections.push(DocumentSectionText {
            source: name,
            index: u32::try_from(index + 1).map_err(failed)?,
            text: bounded,
            scanned_candidate: false,
        });
    }
    Ok(result)
}
fn read_part(part: impl Read, expanded: &mut u64) -> Result<Vec<u8>> {
    let mut xml = Vec::new();
    part.take(MAX_PART + 1)
        .read_to_end(&mut xml)
        .map_err(failed)?;
    *expanded = expanded.saturating_add(xml.len() as u64);
    if xml.len() as u64 > MAX_PART || *expanded > MAX_EXPANDED {
        return Err(Error::extraction_failed("XML expansion limit exceeded"));
    }
    Ok(xml)
}
fn office_parts(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    format: DocumentFormat,
) -> Result<Vec<(String, usize)>> {
    if archive.len() > MAX_ENTRIES {
        return Err(Error::extraction_failed("too many ZIP members"));
    }
    let mut expanded = 0u64;
    let mut parts = Vec::new();
    for i in 0..archive.len() {
        let part = archive.by_index(i).map_err(failed)?;
        expanded = expanded
            .checked_add(part.size())
            .ok_or_else(|| Error::extraction_failed("ZIP size overflow"))?;
        if expanded > MAX_EXPANDED || part.size() > MAX_PART || part.encrypted() {
            return Err(Error::extraction_failed(
                "encrypted ZIP or expansion limit exceeded",
            ));
        }
        let name = part.name();
        if name.len() > 256 {
            return Err(Error::extraction_failed("ZIP member name limit exceeded"));
        }
        let selected = match format {
            DocumentFormat::Docx => name == "word/document.xml",
            DocumentFormat::Pptx => numbered_part(name, "ppt/slides/slide").is_some(),
            DocumentFormat::Xlsx => numbered_part(name, "xl/worksheets/sheet").is_some(),
            DocumentFormat::Pdf => false,
        };
        if selected {
            if parts.iter().any(|(existing, _)| existing == name) {
                return Err(Error::extraction_failed("duplicate document part"));
            }
            parts.push((name.to_owned(), i));
        }
    }
    parts.sort_by_key(|(name, _)| {
        numbered_part(
            name,
            if format == DocumentFormat::Pptx {
                "ppt/slides/slide"
            } else {
                "xl/worksheets/sheet"
            },
        )
        .unwrap_or(1)
    });
    if parts.is_empty() {
        return Err(Error::extraction_failed("document has no supported parts"));
    }
    Ok(parts)
}
fn failed(error: impl std::fmt::Display) -> Error {
    Error::extraction_failed(&error.to_string())
}
fn numbered_part(name: &str, prefix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}
fn bound_text(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
fn xml_strings(xml: &[u8]) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut current = String::new();
    xml_events(xml, |event, text| {
        if let Some(value) = text {
            current.push_str(value);
        }
        if event == "si" {
            result.push(std::mem::take(&mut current));
        }
    })?;
    Ok(result)
}
fn xml_text(xml: &[u8], shared: &[String]) -> Result<String> {
    let mut output = String::new();
    let mut shared_cell = false;
    let mut value = false;
    let mut current = String::new();
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0usize;
    let mut in_text = false;
    loop {
        match reader.read_event().map_err(failed)? {
            Event::Start(e) => {
                depth += 1;
                if depth > 128 {
                    return Err(Error::extraction_failed("XML nesting limit exceeded"));
                }
                let local = e.local_name();
                in_text = local.as_ref() == b"t";
                value = local.as_ref() == b"v";
                if local.as_ref() == b"c" {
                    shared_cell = false;
                    for attr in e.attributes() {
                        let attr = attr.map_err(failed)?;
                        if attr.key.as_ref() == b"t" && attr.value.as_ref() == b"s" {
                            shared_cell = true;
                        }
                    }
                }
            }
            Event::Text(e) if in_text || value => {
                let decoded = e.decode().map_err(failed)?;
                current.push_str(&decoded);
            }
            Event::GeneralRef(e) if in_text || value => {
                let name = e.decode().map_err(failed)?;
                current.push_str(&decode_reference(&name)?);
            }
            Event::CData(e) if in_text || value => {
                current.push_str(&e.decode().map_err(failed)?);
            }
            Event::End(e) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::extraction_failed("invalid XML nesting"))?;
                let local = e.local_name();
                if local.as_ref() == b"t" || local.as_ref() == b"v" {
                    if value && shared_cell {
                        let index: usize = current.parse().map_err(failed)?;
                        output.push_str(shared.get(index).ok_or_else(|| {
                            Error::extraction_failed("invalid shared string index")
                        })?);
                    } else {
                        output.push_str(&current);
                    }
                    current.clear();
                    in_text = false;
                    value = false;
                }
                if [b"p".as_slice(), b"row", b"c"].contains(&local.as_ref()) {
                    output.push('\n');
                }
            }
            Event::DocType(_) => {
                return Err(Error::extraction_failed("XML DTDs are not supported"));
            }
            Event::Eof => {
                if depth != 0 {
                    return Err(Error::extraction_failed("unclosed XML elements"));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(output)
}
fn decode_reference(name: &str) -> Result<String> {
    quick_xml::escape::unescape(&format!("&{name};"))
        .map(std::borrow::Cow::into_owned)
        .map_err(failed)
}
fn xml_events(xml: &[u8], mut consume: impl FnMut(&str, Option<&str>)) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0usize;
    let mut in_text = false;
    loop {
        match reader.read_event().map_err(failed)? {
            Event::Start(e) => {
                depth += 1;
                if depth > 128 {
                    return Err(Error::extraction_failed("XML nesting limit exceeded"));
                }
                in_text = e.local_name().as_ref() == b"t";
            }
            Event::End(e) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::extraction_failed("invalid XML nesting"))?;
                consume(
                    std::str::from_utf8(e.local_name().as_ref()).map_err(failed)?,
                    None,
                );
                in_text = false;
            }
            Event::Text(e) if in_text => consume("", Some(&e.decode().map_err(failed)?)),
            Event::CData(e) if in_text => consume("", Some(&e.decode().map_err(failed)?)),
            Event::Empty(e) if e.local_name().as_ref() == b"si" => consume("si", None),
            Event::GeneralRef(e) if in_text => {
                consume("", Some(&decode_reference(&e.decode().map_err(failed)?)?));
            }
            Event::DocType(_) => {
                return Err(Error::extraction_failed("XML DTDs are not supported"));
            }
            Event::Eof => {
                if depth != 0 {
                    return Err(Error::extraction_failed("unclosed XML elements"));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}
fn extract_pdf(bytes: &[u8], spec: &ExtractDocumentSpec) -> Result<ExtractedDocument> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(Error::invalid_input("bytes", "missing PDF signature"));
    }
    let document = pdf_extract::Document::load_mem(bytes).map_err(failed)?;
    if document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(Error::extraction_failed("encrypted PDF is not supported"));
    }
    let pages = document.get_pages();
    if pages.len() > 4096 {
        return Err(Error::extraction_failed("PDF page limit exceeded"));
    }
    let mut result = ExtractedDocument {
        format: DocumentFormat::Pdf,
        section_count: u32::try_from(pages.len()).map_err(failed)?,
        sections: Vec::new(),
        truncated: pages.len() > spec.max_sections as usize,
    };
    let mut remaining = spec.max_text_bytes as usize;
    for page in pages.keys().take(spec.max_sections as usize) {
        let mut sink = TextSink {
            text: String::new(),
            limit: remaining,
            truncated: false,
        };
        let writer: &mut dyn std::io::Write = &mut sink;
        pdf_extract::output_doc_page(
            &document,
            &mut pdf_extract::PlainTextOutput::new(writer),
            *page,
        )
        .map_err(failed)?;
        remaining -= sink.text.len();
        result.truncated |= sink.truncated;
        let scanned_candidate = sink.text.trim().is_empty() && !sink.truncated;
        result.sections.push(DocumentSectionText {
            source: format!("page:{page}"),
            index: *page,
            text: sink.text,
            scanned_candidate,
        });
    }
    Ok(result)
}
struct TextSink {
    text: String,
    limit: usize,
    truncated: bool,
}
impl std::io::Write for TextSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        let bounded = bound_text(text, self.limit.saturating_sub(self.text.len()));
        self.truncated |= bounded.len() < bytes.len();
        self.text.push_str(&bounded);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
