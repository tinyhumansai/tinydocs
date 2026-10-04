//! Manifest parsing and relationship paths remain bounded and package-local.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

#[test]
fn resolves_normalized_internal_targets_and_rejects_external_or_escaping_paths() {
    for (target, expected) in [
        ("./worksheets/sheet1.xml", "xl/worksheets/sheet1.xml"),
        ("/xl/worksheets/sheet1.xml", "xl/worksheets/sheet1.xml"),
        ("../xl/worksheets/sheet1.xml", "xl/worksheets/sheet1.xml"),
    ] {
        assert_eq!(resolve_target("xl", target).unwrap(), expected);
    }
    for target in [
        "https://example.com/s.xml",
        "../../escape.xml",
        "../",
        "s.xml?query",
        "s.xml#part",
        "a\\b",
        "a\0b",
    ] {
        assert!(resolve_target("xl", target).is_err(), "{target}");
    }
    assert!(resolve_target("xl", &"x".repeat(257)).is_err());
}

#[test]
fn manifest_reader_caps_depth_names_attributes_and_rejects_invalid_structure() {
    let read = |xml: &str| elements(xml.as_bytes(), |_, _, _| Ok(()));
    assert!(read(&format!("{}{}", "<a>".repeat(129), "</a>".repeat(129))).is_err());
    assert!(read(&format!("<{} />", "a".repeat(257))).is_err());
    assert!(read(&format!("<a key='{}'/>", "v".repeat(257))).is_err());
    assert!(read(&format!("<a {}='v'/>", "k".repeat(257))).is_err());
    let mut attrs = String::new();
    for i in 0..33 {
        use std::fmt::Write as _;
        write!(&mut attrs, " k{i}='v'").unwrap();
    }
    assert!(read(&format!("<a{attrs}/>")).is_err());
    for malformed in [
        "<a>",
        "</a>",
        "<a></b>",
        "<!DOCTYPE a><a/>",
        "<a key='&unknown;'/>",
        "<a key='1' key='2'/>",
    ] {
        assert!(read(malformed).is_err(), "{malformed}");
    }
    let mut records = Vec::new();
    elements(
        b"<sheets><!--note--><sheet r:id='a&amp;b'/><sheet r:id='c'></sheet></sheets>",
        |name, parent, attrs| {
            if name == "sheet" {
                records.push((parent.to_owned(), attrs[0].1.clone()));
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        records,
        vec![
            ("sheets".into(), "a&b".into()),
            ("sheets".into(), "c".into())
        ]
    );
}

#[test]
fn manifest_reader_accepts_utf16_xml() {
    let mut xml = vec![0xFF, 0xFE];
    for unit in
        "<?xml version='1.0' encoding='UTF-16'?><root><item key='ok'/></root>".encode_utf16()
    {
        xml.extend_from_slice(&unit.to_le_bytes());
    }
    let mut values = Vec::new();
    elements(&xml, |name, parent, attrs| {
        if name == "item" {
            values.push((parent.to_owned(), attrs[0].1.clone()));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(values, vec![("root".into(), "ok".into())]);
}
