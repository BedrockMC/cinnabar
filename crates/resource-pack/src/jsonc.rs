//! Bedrock-style JSON normalization: comments and trailing commas are accepted.

/// Returns strict JSON bytes for Bedrock JSONC input, or `None` when the text is
/// not UTF-8, has an unterminated string or block comment, or is not one JSON value.
#[must_use]
pub fn normalize_jsonc(bytes: &[u8]) -> Option<Vec<u8>> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    std::str::from_utf8(bytes).ok()?;
    let stripped = strip_comments(bytes)?;
    let normalized = strip_trailing_commas(&stripped);
    serde_json::from_slice::<serde::de::IgnoredAny>(&normalized).ok()?;
    Some(normalized)
}

fn strip_comments(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
        } else if byte == b'"' {
            in_string = true;
            output.push(byte);
            index += 1;
        } else if bytes.get(index..index + 2) == Some(b"//") {
            index += 2;
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                index += 1;
            }
            output.push(b' ');
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            index += 2;
            loop {
                if index >= bytes.len() {
                    return None;
                }
                if bytes.get(index..index + 2) == Some(b"*/") {
                    index += 2;
                    break;
                }
                index += 1;
            }
            output.push(b' ');
        } else {
            output.push(byte);
            index += 1;
        }
    }
    (!in_string).then_some(output)
}

fn strip_trailing_commas(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b','
            && bytes[index + 1..]
                .iter()
                .find(|next| !next.is_ascii_whitespace())
                .is_some_and(|next| matches!(next, b'}' | b']'))
        {
            continue;
        }
        output.push(byte);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::normalize_jsonc;

    #[test]
    fn accepts_comments_trailing_commas_and_bom_but_not_two_values() {
        let text = b"\xef\xbb\xbf{\"a\": [1, 2,], /* c */ \"b\": \"x // y,}\", // z\n}";
        let normalized = normalize_jsonc(text).expect("jsonc");
        let value: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(value["a"], serde_json::json!([1, 2]));
        assert_eq!(value["b"], "x // y,}");
        assert!(normalize_jsonc(b"{} {}").is_none());
        assert!(normalize_jsonc(b"{\"a\": 1 /* open").is_none());
        assert!(normalize_jsonc(b"\xff").is_none());
    }
}
