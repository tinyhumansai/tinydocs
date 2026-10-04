//! Resolve displayed slide and worksheet order through OOXML manifests.

use super::{
    DocumentFormat, MAX_ENTRIES, Result, failed, read_part, zip_admission::AdmittedReader,
};
use crate::Error;
use quick_xml::{Reader, events::Event};
use std::collections::{BTreeMap, BTreeSet};

/// Return only referenced parts, in presentation/workbook order.
pub(super) fn ordered_parts(
    archive: &mut zip::ZipArchive<AdmittedReader<'_>>,
    format: DocumentFormat,
    parts: &[(String, usize)],
    expanded: &mut u64,
) -> Result<Vec<(String, usize)>> {
    let (manifest, relationships, directory, item, container, kind) = match format {
        DocumentFormat::Pptx => (
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt",
            "sldId",
            "sldIdLst",
            "slide",
        ),
        DocumentFormat::Xlsx => (
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "xl",
            "sheet",
            "sheets",
            "worksheet",
        ),
        _ => return Err(invalid("unsupported ordered document format")),
    };
    let manifest_xml = read_part(archive.by_name(manifest).map_err(failed)?, expanded)?;
    let relationships_xml = read_part(archive.by_name(relationships).map_err(failed)?, expanded)?;
    let mut ids = Vec::new();
    elements(&manifest_xml, |name, parent, attrs| {
        if name == item && parent == container {
            let id = attrs
                .iter()
                .find(|(key, _)| key.ends_with(":id"))
                .ok_or_else(|| invalid("missing document relationship id"))?
                .1
                .clone();
            if ids.len() >= MAX_ENTRIES {
                return Err(invalid("too many document references"));
            }
            ids.push(id);
        }
        Ok(())
    })?;
    let mut targets = BTreeMap::new();
    elements(&relationships_xml, |name, parent, attrs| {
        if name != "Relationship" || parent != "Relationships" {
            return Ok(());
        }
        let get = |key: &str| {
            attrs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        let id = get("Id").ok_or_else(|| invalid("missing relationship id"))?;
        let relevant = get("Type").is_some_and(|value| value.rsplit('/').next() == Some(kind));
        let target = if relevant && get("TargetMode") != Some("External") {
            Some(resolve_target(
                directory,
                get("Target").ok_or_else(|| invalid("missing relationship target"))?,
            )?)
        } else {
            None
        };
        if targets.len() >= MAX_ENTRIES || targets.insert(id.to_owned(), target).is_some() {
            return Err(invalid("duplicate or excessive relationships"));
        }
        Ok(())
    })?;
    let available: BTreeMap<_, _> = parts
        .iter()
        .map(|(name, index)| (name.as_str(), *index))
        .collect();
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for id in ids {
        let target = targets
            .get(&id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("missing, external or unsupported document relationship"))?;
        let index = available
            .get(target.as_str())
            .ok_or_else(|| invalid("referenced document part is missing"))?;
        if !seen.insert(target.clone()) {
            return Err(invalid("duplicate document part reference"));
        }
        ordered.push((target.clone(), *index));
    }
    Ok(ordered)
}

fn invalid(reason: &str) -> Error {
    Error::extraction_failed(reason)
}

fn resolve_target(directory: &str, target: &str) -> Result<String> {
    if target.contains(['\\', ':', '?', '#', '\0']) {
        return Err(invalid("invalid document relationship target"));
    }
    let mut components = if target.starts_with('/') {
        Vec::new()
    } else {
        vec![directory]
    };
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components
                    .pop()
                    .ok_or_else(|| invalid("relationship escapes package"))?;
            }
            _ => components.push(component),
        }
    }
    let path = components.join("/");
    if path.is_empty() || path.len() > 256 {
        return Err(invalid("invalid document relationship target"));
    }
    Ok(path)
}

/// Stream bounded manifest metadata, including self-closing elements.
fn elements(
    xml: &[u8],
    mut consume: impl FnMut(&str, &str, &[(String, String)]) -> Result<()>,
) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    let mut parents = Vec::<String>::new();
    loop {
        let event = reader.read_event().map_err(failed)?;
        let is_empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let local = element.local_name();
                let name = std::str::from_utf8(local.as_ref()).map_err(failed)?;
                if name.len() > 256 || parents.len() >= 128 {
                    return Err(invalid("XML nesting or name limit exceeded"));
                }
                let name = name.to_owned();
                let mut attrs = Vec::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(failed)?;
                    if attrs.len() >= 32
                        || attribute.key.as_ref().len() > 256
                        || attribute.value.len() > 256
                    {
                        return Err(invalid("XML metadata limit exceeded"));
                    }
                    attrs.push((
                        std::str::from_utf8(attribute.key.as_ref())
                            .map_err(failed)?
                            .to_owned(),
                        attribute
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .map_err(failed)?
                            .into_owned(),
                    ));
                }
                consume(&name, parents.last().map_or("", String::as_str), &attrs)?;
                if !is_empty {
                    parents.push(name);
                }
            }
            Event::End(_) => {
                parents
                    .pop()
                    .ok_or_else(|| invalid("invalid XML nesting"))?;
            }
            Event::DocType(_) => return Err(invalid("XML DTDs are not supported")),
            Event::Eof => {
                if !parents.is_empty() {
                    return Err(invalid("unclosed XML elements"));
                }
                return Ok(());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "office_order_tests.rs"]
mod tests;
