#![no_main]

use libfuzzer_sys::fuzz_target;
use rusmes_proto::mime::{parse_headers, split_multipart, ContentType, decode_base64, decode_quoted_printable};

fuzz_target!(|data: &[u8]| {
    // Test header parsing
    let _ = parse_headers(data);

    // Test Content-Type parsing
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = ContentType::parse(s);

        // Test multipart with various boundaries
        if s.len() < 100 {
            let boundary = format!("boundary_{}", s.replace('\n', "_").replace('\r', "_"));
            let multipart_msg = format!(
                "--{}\r\nContent-Type: text/plain\r\n\r\nPart1\r\n--{}\r\nContent-Type: text/html\r\n\r\nPart2\r\n--{}--\r\n",
                boundary, boundary, boundary
            );
            let _ = split_multipart(multipart_msg.as_bytes(), &boundary);

            // Test nested multipart
            let inner_boundary = format!("inner_{}", boundary);
            let nested = format!(
                "--{}\r\nContent-Type: multipart/mixed; boundary=\"{}\"\r\n\r\n--{}\r\nPart\r\n--{}--\r\n--{}--\r\n",
                boundary, inner_boundary, inner_boundary, inner_boundary, boundary
            );
            let _ = split_multipart(nested.as_bytes(), &boundary);
        }

        // Test long header lines (folding)
        if s.len() < 50 {
            let long_header = format!("Subject: {}\r\n {}\r\n {}\r\n\r\n", s, s, s);
            let _ = parse_headers(long_header.as_bytes());
        }
    }

    // Test Base64 decoding
    let _ = decode_base64(data);

    // Test Quoted-Printable decoding
    let _ = decode_quoted_printable(data);

    // Test binary data in text parts
    let mixed_data = [data, b"\r\n\r\n", data].concat();
    let _ = parse_headers(&mixed_data);

    // Test extremely long lines (potential DoS)
    if data.len() > 0 && data.len() < 256 {
        let mut long_line = Vec::new();
        for _ in 0..100 {
            long_line.extend_from_slice(data);
        }
        let _ = parse_headers(&long_line);
    }
});
