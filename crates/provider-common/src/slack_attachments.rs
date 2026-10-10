//! Slack `event.files[]` -> fetch references (master plan contract C1).
//!
//! Shared by the provider `ingest_http` and the legacy `messaging-ingress-slack`
//! so the host rule cannot drift. Only the bot token's secret NAME travels.

use greentic_types::ChannelMessageEnvelope;
use serde_json::Value;

use crate::attachment_fetch::{
    FetchRef, PendingAttachment, add_dropped, add_dropped_value, apply_fetch_refs, apply_to_value,
};
use crate::telemetry;

/// The only host a Slack file URL may name (the host sends the bot token there).
pub const SLACK_FILE_HOST: &str = "files.slack.com";
/// Secret NAME of the bot token the host downloads with.
pub const SLACK_TOKEN_SECRET: &str = "SLACK_BOT_TOKEN";

/// Fetch refs plus the file entries that could not become one.
#[derive(Debug, Default)]
pub struct SlackFiles {
    pub pending: Vec<PendingAttachment>,
    pub skipped: usize,
}

/// `https://files.slack.com[/...]` exactly: lowercase, no port, no userinfo,
/// no trailing dot, no other host (not even another `*.slack.com`).
pub fn is_slack_file_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority == SLACK_FILE_HOST)
}

/// `files[]` of a Slack message; entries without a usable url (or type) are counted.
pub fn slack_files(payload: &Value) -> SlackFiles {
    let mut out = SlackFiles::default();
    let Some(files) = payload.get("files").and_then(Value::as_array) else {
        return out;
    };
    for f in files {
        let url = f
            .get("url_private_download")
            .or_else(|| f.get("url_private"))
            .and_then(Value::as_str)
            .filter(|url| is_slack_file_url(url));
        let mime = f.get("mimetype").and_then(Value::as_str);
        let (Some(url), Some(mime)) = (url, mime) else {
            warn_skipped();
            out.skipped += 1;
            continue;
        };
        out.pending.push(PendingAttachment {
            mime_type: mime.to_string(),
            name: f.get("name").and_then(Value::as_str).map(str::to_string),
            size_bytes: f.get("size").and_then(Value::as_u64),
            fetch: FetchRef::Bearer {
                url: url.to_string(),
                secret_key: SLACK_TOKEN_SECRET.to_string(),
            },
            inline_base64: None,
        });
    }
    out
}

/// Files onto a typed envelope, skipped entries counted in `attachments_dropped`.
pub fn apply_slack_files(envelope: &mut ChannelMessageEnvelope, payload: &Value) {
    let files = slack_files(payload);
    apply_fetch_refs(envelope, files.pending);
    add_dropped(envelope, files.skipped);
}

/// [`apply_slack_files`] for an envelope held as raw JSON.
pub fn apply_slack_files_to_value(envelope: &mut Value, payload: &Value) {
    let files = slack_files(payload);
    apply_to_value(envelope, files.pending);
    add_dropped_value(envelope, files.skipped);
}

fn warn_skipped() {
    // Never the url: only a fixed reason.
    telemetry::log(
        telemetry::Level::Warn,
        "attachment dropped at the provider edge",
        &[
            telemetry::Field {
                key: telemetry::field::PROVIDER,
                value: "slack",
            },
            telemetry::Field {
                key: "reason",
                value: "slack file without a files.slack.com url or a type",
            },
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_exact_lowercase_https_files_host_is_accepted() {
        for ok in [
            "https://files.slack.com/files-pri/T1-F1/download/a.png",
            "https://files.slack.com/x?y=1",
            "https://files.slack.com",
        ] {
            assert!(is_slack_file_url(ok), "{ok}");
        }
        for bad in [
            "https://files.slack.com.evil.test/a",
            "https://evilfiles.slack.com/a",
            "https://edge.slack.com/a",
            "https://slack.com/a",
            "https://a.files.slack.com/a",
            "https://slack-files.com/a",
            "https://files.slack.com./a",
            "https://files.slack.com:443/a",
            "https://user@files.slack.com/a",
            "https://files.slack.com@evil.test/a",
            "http://files.slack.com/a",
            "HTTPS://files.slack.com/a",
            "https://FILES.SLACK.COM/a",
            "https://Files.slack.com/a",
            "//files.slack.com/a",
            "files.slack.com/a",
            "",
        ] {
            assert!(!is_slack_file_url(bad), "{bad}");
        }
    }

    fn file(url: Option<&str>, mime: Option<&str>) -> Value {
        let mut f = json!({"id": "F", "name": "a.png", "size": 5});
        if let Some(url) = url {
            f["url_private_download"] = json!(url);
        }
        if let Some(mime) = mime {
            f["mimetype"] = json!(mime);
        }
        f
    }

    #[test]
    fn files_become_bearer_refs_and_unusable_entries_are_counted() {
        let payload = json!({"files": [
            file(Some("https://files.slack.com/a.png"), Some("image/png")),
            file(Some("https://edge.slack.com/b.png"), Some("image/png")),
            file(None, Some("image/png")),
            file(Some("https://files.slack.com/c.png"), None),
            {"id": "G", "name": "d.png", "mimetype": "image/png",
             "url_private": "https://files.slack.com/d.png"},
        ]});
        let out = slack_files(&payload);
        assert_eq!(out.skipped, 3);
        assert_eq!(out.pending.len(), 2);
        assert_eq!(
            out.pending[0].fetch,
            FetchRef::Bearer {
                url: "https://files.slack.com/a.png".into(),
                secret_key: SLACK_TOKEN_SECRET.into()
            }
        );
        assert_eq!(out.pending[0].size_bytes, Some(5));
        assert_eq!(out.pending[1].name.as_deref(), Some("d.png"));
    }

    #[test]
    fn a_message_without_files_counts_nothing() {
        assert_eq!(slack_files(&json!({"text": "hi"})).skipped, 0);
        assert_eq!(slack_files(&json!({"files": "nope"})).skipped, 0);
    }
}
