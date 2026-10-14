//! Ambient capabilities (clock, randomness, sleeping, outbound HTTP).

/// Result of one outbound webhook attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostOutcome {
    /// 2xx.
    Delivered,
    /// 5xx, timeout or transport error: worth retrying.
    Retryable,
    /// 4xx: the receiver refused it; retrying cannot help.
    Rejected,
}

pub trait Env {
    /// Current time, RFC 3339 UTC with second precision (`2026-10-14T10:00:00Z`).
    fn now_rfc3339(&self) -> String;
    /// Current time as unix seconds, used for the webhook timestamp header.
    fn now_unix(&self) -> u64;
    /// A fresh unguessable token made of `[a-z0-9]`.
    fn random_id(&mut self) -> String;
    fn sleep_ms(&mut self, ms: u64);
    fn post(&mut self, url: &str, headers: &[(String, String)], body: &[u8]) -> PostOutcome;
}

#[cfg(test)]
pub mod fake {
    use super::*;

    pub type RecordedPost = (String, Vec<(String, String)>, Vec<u8>);

    #[derive(Default)]
    pub struct FakeEnv {
        pub tick: u64,
        pub ids: u32,
        pub sleeps: Vec<u64>,
        pub outcomes: Vec<PostOutcome>,
        pub posts: Vec<RecordedPost>,
    }

    impl Env for FakeEnv {
        fn now_rfc3339(&self) -> String {
            "2026-10-14T10:00:00Z".to_string()
        }
        fn now_unix(&self) -> u64 {
            1_786_000_000 + self.tick
        }
        fn random_id(&mut self) -> String {
            self.ids += 1;
            format!("rnd{:04}", self.ids)
        }
        fn sleep_ms(&mut self, ms: u64) {
            self.sleeps.push(ms);
        }
        fn post(&mut self, url: &str, headers: &[(String, String)], body: &[u8]) -> PostOutcome {
            self.posts
                .push((url.to_string(), headers.to_vec(), body.to_vec()));
            if self.outcomes.is_empty() {
                PostOutcome::Delivered
            } else {
                self.outcomes.remove(0)
            }
        }
    }
}
