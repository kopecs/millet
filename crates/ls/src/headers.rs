//! Parsing for LSP headers.

pub fn content_length(bs: &[u8]) -> Option<usize> {
  let mut i = bs.iter().position(|&b| b == b':')?;
  if &bs[0..i] != b"Content-Length" {
    return None;
  }
  i += 1;
  while *bs.get(i)? == b' ' {
    i += 1;
  }
  let start = i;
  while bs.get(i)?.is_ascii_digit() {
    i += 1;
  }
  if start == i || *bs.get(i)? != b'\r' || *bs.get(i + 1)? != b'\n' || i + 2 != bs.len() {
    return None;
  }
  let num = std::str::from_utf8(&bs[start..i]).ok()?;
  Some(num.parse().unwrap())
}

#[test]
fn test_content_length() {
  assert_eq!(content_length(b""), None);
  assert_eq!(content_length(b"Content-Length: 123"), None);
  assert_eq!(content_length(b"Content-Length: \r\n 123\r\n"), None);
  assert_eq!(content_length(b"Content-Length: 123\r\n\r\n"), None);
  assert_eq!(content_length(b"Content-Length 123\r\n"), None);
  assert_eq!(content_length(b"Content-Length: 123\n"), None);
  assert_eq!(content_length(b"Content-Length: 123\r"), None);
  assert_eq!(content_length(b"Content-Length:123\r\n"), Some(123));
  assert_eq!(content_length(b"Content-Length: 456\r\n"), Some(456));
  assert_eq!(content_length(b" Content-Length: 123\r\n"), None);
  assert_eq!(content_length(b"Content-Length:    789\r\n"), Some(789));
  assert_eq!(content_length(b"Content-Type: 123\r\n"), None);
  assert_eq!(content_length(b"Content-Length: \r\n"), None);
}
