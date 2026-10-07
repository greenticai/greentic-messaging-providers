//! Upload envelopes: bytes come from re-running the route's pure `parse_upload`.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::HttpInV1;
use provider_common::attachment_fetch::{FetchRef, PendingAttachment};

use crate::directline::upload::{UploadedFile, ValidatedUpload, parse_upload};

/// `/v3/directline/conversations/{id}/upload`, nothing more or less.
pub(super) fn is_upload_path(dl_path: &str) -> bool {
    matches!(
        dl_path.split('/').collect::<Vec<_>>().as_slice(),
        ["", "v3", "directline", "conversations", id, "upload"] if !id.is_empty()
    )
}

/// The upload as the route accepted it (same function, same input).
pub(super) fn validated_upload(request: &HttpInV1) -> Option<ValidatedUpload> {
    parse_upload(&request.headers, &request.body_b64).ok()
}

/// Accepted files as inline fetch refs, in part order.
pub(super) fn pending_from_upload(files: Vec<UploadedFile>) -> Vec<PendingAttachment> {
    files
        .into_iter()
        .map(|file| PendingAttachment {
            mime_type: file.content_type.to_string(),
            size_bytes: Some(file.data.len() as u64),
            inline_base64: Some(STANDARD.encode(&file.data)),
            name: file.name,
            fetch: FetchRef::Inline,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::ingest::stamp_ingest_envelopes;
    use super::*;
    use crate::directline::upload::parse_upload;
    use base64::engine::general_purpose::STANDARD;
    use greentic_types::messaging::universal_dto::{Header, HttpOutV1};
    use serde_json::{Value, json};

    const PATH: &str = "/v3/directline/conversations/conv-1/upload";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nDATA";

    fn file_part(filename: &str, ct: &str, bytes: &[u8]) -> Vec<u8> {
        let mut b = format!(
            "--BB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {ct}\r\n\r\n"
        )
        .into_bytes();
        b.extend_from_slice(bytes);
        b.extend_from_slice(b"\r\n");
        b
    }

    fn activity_part(json: &str) -> Vec<u8> {
        format!("--BB\r\nContent-Disposition: form-data; name=\"activity\"\r\n\r\n{json}\r\n")
            .into_bytes()
    }

    fn request(parts: &[Vec<u8>]) -> HttpInV1 {
        let mut body = parts.concat();
        body.extend_from_slice(b"--BB--\r\n");
        HttpInV1 {
            method: "POST".into(),
            path: format!("/messaging/ingress/webchat/default/_{PATH}"),
            query: Some("userId=alice".into()),
            headers: vec![Header {
                name: "Content-Type".into(),
                value: "multipart/form-data; boundary=BB".into(),
            }],
            body_b64: STANDARD.encode(body),
            route_hint: None,
            binding_id: None,
            config: None,
        }
    }

    fn accepted() -> HttpOutV1 {
        let headers = [
            ("X-Greentic-Env", "prod"),
            ("X-Greentic-Tenant", "acme"),
            ("X-Greentic-User", "alice"),
            ("X-Greentic-User-Verified", "false"),
        ]
        .into_iter()
        .map(|(name, value)| Header {
            name: name.into(),
            value: value.into(),
        })
        .collect();
        HttpOutV1 {
            status: 201,
            headers,
            body_b64: String::new(),
            events: Vec::new(),
        }
    }

    fn stamp(req: &HttpInV1, mut out: HttpOutV1) -> HttpOutV1 {
        stamp_ingest_envelopes(req, PATH, &mut out);
        out
    }

    #[test]
    fn an_upload_puts_inline_bytes_on_the_envelope() {
        let req = request(&[
            activity_part(r#"{"text":"look","value":{"step":"x"}}"#),
            file_part("p.png", "image/png", PNG),
        ]);
        let out = stamp(&req, accepted());
        assert_eq!(out.events.len(), 1);
        let env = &out.events[0];
        assert_eq!(env.text.as_deref(), Some("look"));
        assert_eq!(env.from.as_ref().map(|a| a.id.as_str()), Some("alice"));
        assert_eq!(env.tenant.tenant_id.as_str(), "acme");
        assert_eq!(env.attachments.len(), 1);
        let a = &env.attachments[0];
        assert_eq!(a.mime_type, "image/png");
        assert_eq!(a.url, None);
        assert_eq!(a.name.as_deref(), Some("p.png"));
        assert_eq!(a.size_bytes, Some(PNG.len() as u64));
        assert_eq!(
            a.content,
            Some(json!({"data_base64": STANDARD.encode(PNG)}))
        );
        assert_eq!(
            env.extensions["attachment_fetch"],
            json!([{"kind": "inline"}])
        );
        assert!(!env.extensions.contains_key("attachments"));
        assert!(!env.metadata.contains_key("greentic_submit"));
    }

    #[test]
    fn several_files_stay_index_parallel_and_in_part_order() {
        let req = request(&[
            file_part("a.png", "image/png", PNG),
            file_part("b.pdf", "application/pdf", b"%PDF-1.7 x"),
            file_part("c.csv", "text/csv", b"a,b\n1,2\n"),
        ]);
        let env = &stamp(&req, accepted()).events[0];
        let mimes: Vec<&str> = env
            .attachments
            .iter()
            .map(|a| a.mime_type.as_str())
            .collect();
        assert_eq!(mimes, ["image/png", "application/pdf", "text/csv"]);
        assert_eq!(
            env.extensions["attachment_fetch"],
            json!([{"kind": "inline"}, {"kind": "inline"}, {"kind": "inline"}])
        );
        assert_eq!(
            env.attachments[1].content,
            Some(json!({"data_base64": STANDARD.encode(b"%PDF-1.7 x")}))
        );
        assert_eq!(env.text.as_deref(), Some(""));
    }

    #[test]
    fn the_envelope_carries_exactly_what_the_route_validated() {
        let bodies: Vec<Vec<Vec<u8>>> = vec![
            vec![file_part("a.png", "image/png", b"\xff\xd8\xff\xe0JFIF")],
            vec![
                file_part("a.png", "image/png", PNG),
                file_part("a.png", "image/png", PNG),
            ],
            vec![
                file_part("x", "application/json", b"{\"k\":1}"),
                file_part("y", "application/json", b"{bad"),
            ],
            vec![
                file_part("a.png", "image/png", PNG),
                b"--BB--\r\n".to_vec(),
                file_part("late.png", "image/png", PNG),
            ],
        ];
        for parts in bodies {
            let req = request(&parts);
            let routed = parse_upload(&req.headers, &req.body_b64).expect("route accepts");
            let env = &stamp(&req, accepted()).events[0];
            let from_env: Vec<(String, Option<String>, Option<u64>)> = env
                .attachments
                .iter()
                .map(|a| (a.mime_type.clone(), a.name.clone(), a.size_bytes))
                .collect();
            let from_route: Vec<(String, Option<String>, Option<u64>)> = routed
                .files
                .iter()
                .map(|f| {
                    (
                        f.content_type.to_string(),
                        f.name.clone(),
                        Some(f.data.len() as u64),
                    )
                })
                .collect();
            assert_eq!(from_env, from_route);
        }
    }

    #[test]
    fn a_refused_or_unparseable_upload_emits_nothing() {
        let req = request(&[file_part("p.png", "image/png", PNG)]);
        let mut refused = accepted();
        refused.status = 415;
        assert!(stamp(&req, refused).events.is_empty());
        let mut broken = req.clone();
        broken.body_b64 = STANDARD.encode(b"--BB\r\nbroken");
        assert!(stamp(&broken, accepted()).events.is_empty());
    }

    #[test]
    fn only_the_exact_upload_path_is_an_upload() {
        assert!(is_upload_path(PATH));
        for other in [
            "/v3/directline/conversations/conv-1/activities",
            "/v3/directline/conversations/upload",
            "/v3/directline/conversations/a/b/upload",
            "/v3/directline/conversations/conv-1/upload/x",
            "/v3/directline/conversations//upload",
        ] {
            assert!(!is_upload_path(other), "{other}");
        }
    }

    #[test]
    fn pending_items_are_inline_refs_with_their_bytes() {
        let req = request(&[file_part("p.png", "image/png", PNG)]);
        let upload = validated_upload(&req).expect("valid");
        let pending = pending_from_upload(upload.files);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].mime_type, "image/png");
        assert_eq!(
            pending[0].fetch,
            provider_common::attachment_fetch::FetchRef::Inline
        );
        assert_eq!(
            pending[0].inline_base64.as_deref(),
            Some(STANDARD.encode(PNG).as_str())
        );
        assert!(!format!("{:?}", pending[0]).contains(&STANDARD.encode(PNG)));
    }

    #[test]
    fn client_supplied_content_urls_on_activities_never_become_fetch_refs() {
        let path = "/v3/directline/conversations/conv-1/activities";
        let body = json!({"type":"message","text":"x","attachments":[{"contentType":"image/png","contentUrl":"http://169.254.169.254/x"}]});
        let req = HttpInV1 {
            method: "POST".into(),
            path: path.into(),
            query: None,
            headers: vec![],
            body_b64: STANDARD.encode(serde_json::to_vec(&body).expect("json")),
            route_hint: None,
            binding_id: None,
            config: None,
        };
        let mut out = accepted();
        stamp_ingest_envelopes(&req, path, &mut out);
        let env = &out.events[0];
        assert!(env.attachments.is_empty());
        assert!(!env.extensions.contains_key("attachment_fetch"));
        assert_eq!(
            env.extensions["attachments"][0]["contentUrl"],
            Value::from("http://169.254.169.254/x")
        );
    }
}
