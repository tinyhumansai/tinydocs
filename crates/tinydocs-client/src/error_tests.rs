//! Formatting behavior for `TinyDocs` errors, including message redaction.

use super::*;

#[test]
fn debug_and_display_redact_module_messages_but_raw_message_is_explicitly_available() {
    let errors = [
        DocumentCallError::Unavailable("module secret".to_string()),
        DocumentCallError::InvalidInput("field must be present".to_string()),
        DocumentCallError::Failed("remote detail".to_string()),
    ];
    for error in errors {
        let debug = format!("{error:?}");
        let display = error.to_string();
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("present"));
        assert!(!debug.contains("remote detail"));
        assert!(!display.contains("secret"));
        assert!(!display.contains("present"));
        assert!(!display.contains("remote detail"));
    }
    let invalid = DocumentCallError::InvalidInput("field must be present".to_string());
    assert_eq!(invalid.raw_message(), "field must be present");
}
