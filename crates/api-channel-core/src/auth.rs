//! Bearer verification.

use crate::http::HttpRequest;
use sha2::{Digest, Sha256};

/// Compare two secrets without leaking where they differ or how long they
/// are: both are hashed to a fixed width first, then folded byte by byte.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let da = Sha256::digest(a.as_bytes());
    let db = Sha256::digest(b.as_bytes());
    da.iter()
        .zip(db.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// True only when the request carries `Authorization: Bearer <expected>`.
/// An unset `expected` refuses everything (fail closed).
pub fn bearer_ok(req: &HttpRequest, expected: Option<&str>) -> bool {
    let Some(expected) = expected.filter(|k| !k.is_empty()) else {
        return false;
    };
    let Some(value) = req.header("authorization") else {
        return false;
    };
    let Some((scheme, token)) = value.trim().split_once(' ') else {
        return false;
    };
    scheme.eq_ignore_ascii_case("bearer") && constant_time_eq(token.trim(), expected)
}
