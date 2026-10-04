//! Deterministic mixed text/image PDF fixtures.
/// Build pages carrying text and/or a genuine embedded raster image.
pub(crate) fn document(pages: &[(&str, bool)], encrypted: bool) -> Vec<u8> {
    let mut objects = vec![
        String::new(),
        String::new(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut kids = Vec::new();
    for (text, image) in pages {
        let page_id = objects.len() + 1;
        kids.push(format!("{page_id} 0 R"));
        let content = if *image {
            "q 100 0 0 100 0 0 cm /Im1 Do Q\n".to_owned()
        } else {
            String::new()
        } + &format!("BT /F1 24 Tf 10 100 Td ({text}) Tj ET\n");
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 3 0 R >> /XObject << /Im1 {} 0 R >> >> /Contents {} 0 R >>", page_id+2, page_id+1));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ));
        objects.push("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 7 /Filter /ASCIIHexDecode >>\nstream\n000000>\nendstream".to_owned());
    }
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".to_owned();
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    let encrypt = if encrypted {
        "/Encrypt << /Filter /Standard /V 1 /R 2 /O (bad) /U (bad) /P -4 >>"
    } else {
        ""
    };
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {encrypt} >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}
