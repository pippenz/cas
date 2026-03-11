use serde::Serialize;
use std::io::Write;
use tiny_http::{Header, Response, StatusCode};

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ErrorBody {
    schema_version: u32,
    error: ErrorInfo,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ErrorInfo {
    code: String,
    message: String,
}

pub(crate) fn json_response<T: Serialize>(
    status: StatusCode,
    body: &T,
    cors_allow_origin: Option<&str>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let json = serde_json::to_vec_pretty(body).unwrap_or_else(|_| b"{}".to_vec());
    json_response_bytes(status, json, cors_allow_origin)
}

pub(crate) fn json_response_bytes(
    status: StatusCode,
    body_bytes: Vec<u8>,
    cors_allow_origin: Option<&str>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = Response::from_data(body_bytes).with_status_code(status);
    resp.add_header(Header::from_bytes("Content-Type", "application/json").unwrap());
    if let Some(origin) = cors_allow_origin {
        resp.add_header(Header::from_bytes("Access-Control-Allow-Origin", origin).unwrap());
        resp.add_header(
            Header::from_bytes(
                "Access-Control-Allow-Headers",
                "authorization, content-type",
            )
            .unwrap(),
        );
        resp.add_header(
            Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS").unwrap(),
        );
    }
    resp
}

pub(crate) fn error_response(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
    cors_allow_origin: Option<&str>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(
        status,
        &ErrorBody {
            schema_version: 1,
            error: ErrorInfo {
                code: code.to_string(),
                message: message.into(),
            },
        },
        cors_allow_origin,
    )
}

pub(crate) fn require_auth(
    req: &tiny_http::Request,
    token: Option<&str>,
    no_auth: bool,
) -> std::result::Result<(), &'static str> {
    if no_auth {
        return Ok(());
    }
    let Some(token) = token else {
        return Err("auth_not_configured");
    };
    let hdr = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .map(|h| h.value.as_str())
        .unwrap_or("");
    let expected = format!("Bearer {token}");
    if hdr == expected {
        Ok(())
    } else {
        Err("unauthorized")
    }
}

pub(crate) fn url_decode(input: &str) -> String {
    // Minimal query decoding: '+' to ' ', and percent-decoding (%HH).
    // This is sufficient for our local bridge query params.
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h1 = bytes[i + 1];
                let h2 = bytes[i + 2];
                let hex = |c: u8| -> Option<u8> {
                    match c {
                        b'0'..=b'9' => Some(c - b'0'),
                        b'a'..=b'f' => Some(10 + (c - b'a')),
                        b'A'..=b'F' => Some(10 + (c - b'A')),
                        _ => None,
                    }
                };
                if let (Some(a), Some(b)) = (hex(h1), hex(h2)) {
                    out.push((a << 4) | b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub(crate) fn parse_u64_query(query: &str, key: &str) -> Option<u64> {
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            let v = url_decode(v);
            if let Ok(n) = v.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

pub(crate) fn parse_string_query(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            let v = url_decode(v);
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

pub(crate) fn parse_bool_query(query: &str, key: &str) -> Option<bool> {
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            let v = url_decode(v);
            return Some(v == "1" || v == "true" || v == "yes");
        }
    }
    None
}

pub(crate) fn write_chunked<W: Write>(w: &mut W, data: &[u8]) -> std::io::Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    write!(w, "{:X}\r\n", data.len())?;
    w.write_all(data)?;
    w.write_all(b"\r\n")?;
    w.flush()
}

pub(crate) fn write_chunked_end<W: Write>(w: &mut W) -> std::io::Result<()> {
    w.write_all(b"0\r\n\r\n")?;
    w.flush()
}
