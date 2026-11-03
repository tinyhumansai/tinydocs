//! Word documents and `PowerPoint` decks: zip archives of XML, walked directly.

use std::fmt::Display;
use std::io::{Cursor, Read, Seek};
use std::str;

use quick_xml::events::Event;
use zip::ZipArchive;

use super::{MAX_DECOMPRESSED_BYTES, unreadable};
use crate::Result;

// ZIP general-purpose bit 11 marks UTF-8 names. Inspect it through zip's
// stable flags value instead of naming its unstable flag enum.
const UTF8_FILENAME_FLAG: u16 = 1 << 11;

/// The body text of a Word document.
///
/// `w:p` is a paragraph and `w:t` a run of text inside it: joining runs
/// without a paragraph break would run every heading into the sentence after
/// it, and chunkers split on paragraphs.
pub(super) fn docx(bytes: &[u8]) -> Result<String> {
    let mut archive = open(bytes, "document")?;
    Ok(read_parts(
        &mut archive,
        &["word/document.xml"],
        "w:p",
        "w:t",
    ))
}

/// The text of every slide in a deck, in slide order.
pub(super) fn pptx(bytes: &[u8]) -> Result<String> {
    let mut archive = open(bytes, "deck")?;
    let mut slides = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index_data(index).map_err(invalid_member_name)?;
        let raw_name = file.name_raw();
        if file.flags().as_u16() & UTF8_FILENAME_FLAG != 0 && str::from_utf8(raw_name).is_err() {
            // ZIP 9 decodes malformed UTF-8 lossily. Detect it from the raw
            // bytes first, but ignore it when the member is unrelated to slides.
            if raw_name.starts_with(b"ppt/slides/slide") {
                return Err(unreadable("the deck contains an invalid member name"));
            }
            continue;
        }
        let name = file.name().map_err(invalid_member_name)?;
        if let Some(number) = name
            .strip_prefix("ppt/slides/slide")
            .and_then(|rest| rest.strip_suffix(".xml"))
            .and_then(|number| number.parse::<u32>().ok())
        {
            slides.push((number, name.into_owned()));
        }
    }
    slides.sort_unstable();
    let slides: Vec<String> = slides.into_iter().map(|(_, name)| name).collect();
    let names: Vec<&str> = slides.iter().map(String::as_str).collect();
    Ok(read_parts(&mut archive, &names, "a:p", "a:t"))
}

/// Opens an Office archive, refusing one whose declared expansion exceeds
/// [`MAX_DECOMPRESSED_BYTES`] before any entry data is read.
///
/// `noun` names the document in the refusal ("the spreadsheet expands…").
pub(super) fn open<'a>(bytes: &'a [u8], noun: &str) -> Result<ZipArchive<Cursor<&'a [u8]>>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| unreadable(&format!("the {noun} is not a readable archive: {error}")))?;
    if declared_uncompressed(&mut archive).is_none_or(|total| total > MAX_DECOMPRESSED_BYTES) {
        return Err(unreadable(&format!(
            "the {noun} expands beyond the size this build can read safely"
        )));
    }
    Ok(archive)
}

/// Sums the uncompressed sizes every entry declares, touching only the
/// central directory — so this stays cheap however far the archive expands.
/// `None` means an entry could not be inspected, which callers refuse.
fn declared_uncompressed<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Option<u64> {
    (0..archive.len()).try_fold(0u64, |total, index| {
        archive
            .by_index_data(index)
            .ok()
            .map(|file| total.saturating_add(file.size()))
    })
}

fn invalid_member_name(error: impl Display) -> crate::Error {
    unreadable(&format!(
        "the deck contains an invalid member name: {error}"
    ))
}

/// Reads the named parts in order and pulls their `text_tag` runs, breaking a
/// paragraph wherever `paragraph_tag` closes. A part that is missing or cannot
/// be read is skipped: the rest of the document is still worth having.
fn read_parts<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    parts: &[&str],
    paragraph_tag: &str,
    text_tag: &str,
) -> String {
    let mut out = String::new();
    for part in parts {
        let Ok(file) = archive.by_name(part) else {
            continue;
        };
        // Capped as well as pre-checked: an archive that declares small sizes
        // but streams more cannot force an unbounded allocation either.
        let mut xml = String::new();
        if file
            .take(MAX_DECOMPRESSED_BYTES)
            .read_to_string(&mut xml)
            .is_err()
        {
            continue;
        }
        out.push_str(&xml_text(&xml, paragraph_tag, text_tag));
    }
    out
}

/// Concatenates every `text_tag` run, with a blank line at each
/// `paragraph_tag` close.
fn xml_text(xml: &str, paragraph_tag: &str, text_tag: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.name().as_ref() == text_tag.as_bytes() => in_text = true,
            Ok(Event::End(tag)) if tag.name().as_ref() == text_tag.as_bytes() => in_text = false,
            Ok(Event::End(tag)) if tag.name().as_ref() == paragraph_tag.as_bytes() => {
                out.push_str("\n\n");
            }
            Ok(Event::Text(text)) if in_text => {
                out.push_str(&text.decode().unwrap_or_default());
            }
            // The parser reports each `&amp;` / `&#38;` as its own event;
            // dropping them would turn "Q&A" into "QA".
            Ok(Event::GeneralRef(reference)) if in_text => {
                if let Ok(Some(ch)) = reference.resolve_char_ref() {
                    out.push(ch);
                } else if let Some(entity) = reference
                    .decode()
                    .ok()
                    .and_then(|name| quick_xml::escape::resolve_predefined_entity(&name))
                {
                    out.push_str(entity);
                }
            }
            // A malformed part yields what was read up to the fault: a
            // truncated document still holds the text before the break.
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
#[path = "ooxml_tests.rs"]
mod tests;
