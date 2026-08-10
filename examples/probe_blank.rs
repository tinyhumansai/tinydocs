use tinydocs::docx::{generate, DocumentSection, DocumentSpec};
fn main() {
    let s = DocumentSpec {
        title: "Trimmed".to_string(),
        author: Some("   ".to_string()),
        sections: vec![DocumentSection {
            heading: Some("Kept".to_string()),
            paragraphs: vec!["real".to_string(), "   ".to_string(), String::new()],
            bullets: vec!["item".to_string(), "\t\n".to_string()],
        }],
    };
    let bytes = generate(&s).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut entry = zip.by_name("word/document.xml").unwrap();
    let mut body = String::new();
    std::io::Read::read_to_string(&mut entry, &mut body).unwrap();
    println!("{body}");
}
