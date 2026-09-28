//! Tolerant reader for the vanilla `ui/*.json` files, which are JSON5-flavoured:
//! `//` and `/* */` comments, trailing commas, and banner characters that live
//! only inside header comments. We strip those to strict JSON and hand the rest
//! to `serde_json`; the vanilla pack needs nothing beyond comments and trailing
//! commas, so single-quote strings and bare keys are deliberately not accepted.

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[error("json parse error: {0}")]
pub struct ParseError(#[from] serde_json::Error);

/// Parse tolerant JSON into a value after stripping comments and trailing commas.
pub fn parse(source: &str) -> Result<Value, ParseError> {
    Ok(serde_json::from_str(&sanitize(source))?)
}

/// Drop comments and commas that precede a closing `]`/`}`. Bytes inside string
/// literals are copied verbatim so multi-byte UTF-8 survives untouched.
fn sanitize(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                out.push(b'"');
                i += 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    out.push(c);
                    i += 1;
                    if c == b'\\' {
                        if i < bytes.len() {
                            out.push(bytes[i]);
                            i += 1;
                        }
                    } else if c == b'"' {
                        break;
                    }
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    // Keep newlines so serde_json error line numbers stay meaningful.
                    if bytes[i] == b'\n' {
                        out.push(b'\n');
                    }
                    i += 1;
                }
                i += 2;
            }
            b',' if next_significant_is_closer(bytes, i + 1) => {
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

fn next_significant_is_closer(bytes: &[u8], mut i: usize) -> bool {
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        return matches!(bytes.get(i), Some(b']') | Some(b'}'));
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn line_comments_and_banner_chars_are_ignored() {
        let value =
            parse("/****\n+* banner *\n****/\n{\n  // a field\n  \"a\": 1 // trailing note\n}")
                .unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn trailing_commas_in_objects_and_arrays_are_tolerated() {
        let value = parse("{ \"list\": [1, 2, 3,], \"obj\": { \"k\": 1, }, }").unwrap();
        assert_eq!(value["list"], serde_json::json!([1, 2, 3]));
        assert_eq!(value["obj"]["k"], 1);
    }

    #[test]
    fn slashes_and_commas_inside_strings_survive() {
        let value = parse("{ \"expr\": \"(not (#a = '') or b),\" }").unwrap();
        assert_eq!(value["expr"], "(not (#a = '') or b),");
    }
}
