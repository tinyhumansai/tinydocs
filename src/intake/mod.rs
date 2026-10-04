//! Bounded document text intake without filesystem or network access.
//!
//! OOXML reads selected ZIP members and parses XML events. PDF parsing is
//! CPU-bound; hosts own execution deadlines and any stronger isolation.
use crate::{Error, Result};
use quick_xml::{Reader, events::Event};
use std::borrow::Cow;
use std::io::Read;
mod office_order;
mod zip_admission;
pub use tinydocs_bus::intake::{
    DocumentFormat, DocumentSectionText, ExtractDocumentSpec, ExtractedDocument,
};

const MAX_INPUT: usize = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 32 * 1024 * 1024;
const MAX_PART: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_SHARED_STRINGS: usize = 65_536;
const MAX_SHARED_TEXT_BYTES: usize = 4 * 1024 * 1024;

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
    zip_admission::zip_preflight(bytes)?;
    let mut archive = zip_admission::open_admitted_zip(bytes)?;
    let mut expanded_read = 0u64;
    let parts = office_parts(&mut archive, spec.format, &mut expanded_read)?;
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
        let text = xml_text(&xml, &shared, remaining)?;
        remaining -= text.text.len();
        result.truncated |= text.truncated;
        result.sections.push(DocumentSectionText {
            source: name,
            index: u32::try_from(index + 1).map_err(failed)?,
            text: text.text,
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
    archive: &mut zip::ZipArchive<zip_admission::AdmittedReader<'_>>,
    format: DocumentFormat,
    expanded_read: &mut u64,
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
            DocumentFormat::Pptx | DocumentFormat::Xlsx => true,
            DocumentFormat::Pdf => false,
        };
        if selected {
            if parts.iter().any(|(existing, _)| existing == name) {
                return Err(Error::extraction_failed("duplicate document part"));
            }
            parts.push((name.to_owned(), i));
        }
    }
    if matches!(format, DocumentFormat::Pptx | DocumentFormat::Xlsx) {
        parts = office_order::ordered_parts(archive, format, &parts, expanded_read)?;
    }
    if parts.is_empty() {
        return Err(Error::extraction_failed("document has no supported parts"));
    }
    Ok(parts)
}
fn failed(error: impl std::fmt::Display) -> Error {
    Error::extraction_failed(&error.to_string())
}
pub(super) fn normalize_xml(xml: &[u8]) -> Result<Cow<'_, [u8]>> {
    let (encoding, content) = if xml.starts_with(&[0xFF, 0xFE]) {
        (Some(false), &xml[2..])
    } else if xml.starts_with(&[0xFE, 0xFF]) {
        (Some(true), &xml[2..])
    } else if xml.starts_with(&[b'<', 0, b'?', 0]) {
        (Some(false), xml)
    } else if xml.starts_with(&[0, b'<', 0, b'?']) {
        (Some(true), xml)
    } else {
        (None, xml)
    };
    let Some(big_endian) = encoding else {
        return Ok(Cow::Borrowed(
            xml.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(xml),
        ));
    };
    if content.len() % 2 != 0 {
        return Err(Error::extraction_failed("invalid UTF-16 XML length"));
    }
    let units = content.chunks_exact(2).map(|pair| {
        if big_endian {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            u16::from_le_bytes([pair[0], pair[1]])
        }
    });
    let mut utf8 = String::new();
    for character in char::decode_utf16(units) {
        utf8.push(character.map_err(failed)?);
    }
    if utf8.starts_with("<?xml") {
        let declaration_end = utf8
            .find("?>")
            .ok_or_else(|| Error::extraction_failed("invalid XML declaration"))?;
        utf8.drain(..declaration_end + 2);
    }
    Ok(Cow::Owned(utf8.into_bytes()))
}
fn xml_strings(xml: &[u8]) -> Result<Vec<String>> {
    let xml = normalize_xml(xml)?;
    let mut result = Vec::new();
    let mut current = String::new();
    let mut text_bytes = 0usize;
    xml_events(xml.as_ref(), |event, text| {
        if let Some(value) = text {
            text_bytes = text_bytes
                .checked_add(value.len())
                .ok_or_else(|| failed("shared string size overflow"))?;
            if text_bytes > MAX_SHARED_TEXT_BYTES {
                return Err(failed("shared string text budget exceeded"));
            }
            current.push_str(value);
        }
        if event == "si" {
            if result.len() >= MAX_SHARED_STRINGS {
                return Err(failed("shared string count budget exceeded"));
            }
            current.shrink_to_fit();
            result.push(std::mem::take(&mut current));
        }
        Ok(())
    })?;
    Ok(result)
}
fn xml_text(xml: &[u8], shared: &[String], limit: usize) -> Result<TextSink> {
    let xml = normalize_xml(xml)?;
    let mut output = TextSink {
        text: String::with_capacity(limit),
        limit,
        truncated: false,
    };
    let mut shared_cell = false;
    let mut value = false;
    let mut current = String::new();
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut depth = 0usize;
    let mut in_text = false;
    let mut phonetic_depth = None;
    loop {
        match reader.read_event().map_err(failed)? {
            Event::Start(e) => {
                depth += 1;
                if depth > 128 {
                    return Err(Error::extraction_failed("XML nesting limit exceeded"));
                }
                let local = e.local_name();
                if local.as_ref() == b"rPh" && phonetic_depth.is_none() {
                    phonetic_depth = Some(depth);
                }
                in_text = local.as_ref() == b"t" && phonetic_depth.is_none();
                if phonetic_depth.is_none() {
                    match local.as_ref() {
                        b"br" | b"cr" => output.append("\n"),
                        b"tab" => output.append("\t"),
                        _ => {}
                    }
                }
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
                output.append_value(&mut current, value && shared_cell, &decoded)?;
            }
            Event::GeneralRef(e) if in_text || value => {
                let name = e.decode().map_err(failed)?;
                let decoded = decode_reference(&name)?;
                output.append_value(&mut current, value && shared_cell, &decoded)?;
            }
            Event::CData(e) if in_text || value => {
                let decoded = e.decode().map_err(failed)?;
                output.append_value(&mut current, value && shared_cell, &decoded)?;
            }
            Event::End(e) => {
                if phonetic_depth == Some(depth) {
                    phonetic_depth = None;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::extraction_failed("invalid XML nesting"))?;
                let local = e.local_name();
                if local.as_ref() == b"t" || local.as_ref() == b"v" {
                    if value && shared_cell {
                        let index: usize = current.parse().map_err(failed)?;
                        output.append(shared.get(index).ok_or_else(|| {
                            Error::extraction_failed("invalid shared string index")
                        })?);
                    }
                    current.clear();
                    in_text = false;
                    value = false;
                }
                if [b"p".as_slice(), b"row", b"c"].contains(&local.as_ref()) {
                    output.append("\n");
                }
            }
            Event::Empty(e) if phonetic_depth.is_none() => match e.local_name().as_ref() {
                b"br" | b"cr" => output.append("\n"),
                b"tab" => output.append("\t"),
                _ => {}
            },
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
    output.text.shrink_to_fit();
    Ok(output)
}
fn append_index(current: &mut String, value: &str) -> Result<()> {
    if current.len().saturating_add(value.len()) > 20 {
        return Err(Error::extraction_failed("shared string index is too long"));
    }
    current.push_str(value);
    Ok(())
}
fn decode_reference(name: &str) -> Result<String> {
    quick_xml::escape::unescape(&format!("&{name};"))
        .map(std::borrow::Cow::into_owned)
        .map_err(failed)
}
fn xml_events(xml: &[u8], mut consume: impl FnMut(&str, Option<&str>) -> Result<()>) -> Result<()> {
    let xml = normalize_xml(xml)?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut depth = 0usize;
    let mut in_text = false;
    let mut phonetic_depth = None;
    loop {
        match reader.read_event().map_err(failed)? {
            Event::Start(e) => {
                depth += 1;
                if depth > 128 {
                    return Err(Error::extraction_failed("XML nesting limit exceeded"));
                }
                let local = e.local_name();
                if local.as_ref() == b"rPh" && phonetic_depth.is_none() {
                    phonetic_depth = Some(depth);
                }
                in_text = local.as_ref() == b"t" && phonetic_depth.is_none();
            }
            Event::End(e) => {
                if phonetic_depth == Some(depth) {
                    phonetic_depth = None;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::extraction_failed("invalid XML nesting"))?;
                consume(
                    std::str::from_utf8(e.local_name().as_ref()).map_err(failed)?,
                    None,
                )?;
                in_text = false;
            }
            Event::Text(e) if in_text => consume("", Some(&e.decode().map_err(failed)?))?,
            Event::CData(e) if in_text => consume("", Some(&e.decode().map_err(failed)?))?,
            Event::Empty(e) if e.local_name().as_ref() == b"si" => consume("si", None)?,
            Event::GeneralRef(e) if in_text => {
                consume("", Some(&decode_reference(&e.decode().map_err(failed)?)?))?;
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
impl TextSink {
    fn append_value(&mut self, index: &mut String, shared: bool, text: &str) -> Result<()> {
        if shared {
            append_index(index, text)
        } else {
            self.append(text);
            Ok(())
        }
    }
    fn append(&mut self, text: &str) {
        if self.truncated {
            return;
        }
        let remaining = self.limit.saturating_sub(self.text.len());
        let mut end = text.len().min(remaining);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.truncated |= end < text.len();
        self.text.push_str(&text[..end]);
    }
}
impl std::io::Write for TextSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        self.append(text);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
