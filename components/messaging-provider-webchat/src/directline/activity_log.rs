// Per-activity storage for Direct Line conversations.
//
// The `greentic:state/state-store` contract offers `read`, `write` and
// `delete` on opaque blobs, plus (from @1.1.0) an atomic `write-if-absent`:
// no list, no append, no TTL, no compare-and-swap.
// A conversation therefore lives under these keys:
//
// ```text
// <conv_key>                    header (ConversationState, layout 2, no activities)
// <conv_key>/act/<seq:020>      one StoredActivity, seq == its watermark
// ```
//
// `<conv_key>` is the key the single-blob layout already used, so a
// conversation stored by an older build is found at the same place and is read
// as-is (`ConversationState::is_legacy`). It is migrated lazily by the next
// append: every legacy activity is copied to its own key, and only then is the
// header rewritten without them. A crash in between leaves the legacy blob
// intact and the migration simply repeats.
//
// # What is and is not guaranteed
//
// The header's `next_watermark` is a hint, not the truth: the activity keys
// are. Readers probe past the hint, so a lost or regressed header write heals
// by itself.
//
// With `write-if-absent` an append claims a slot atomically: the call that
// creates the key owns that watermark and a loser moves to the next slot, so
// two writers can never share one.
//
// Hosts without it (`StateStore::write_if_absent` answering `None`) fall back
// to probing the slot, writing and reading it back. A race remains there: two
// writers whose writes are not interleaved with the other's read-back can each
// believe they won, and the later write replaces the earlier activity.

use super::state::{ConversationState, LAYOUT_PER_ACTIVITY, StoredActivity};
use super::store::StateStore;

/// Most activities returned by one read; clients continue from the returned
/// watermark.
pub const ACTIVITY_PAGE_SIZE: usize = 500;
/// Slots an append probes before giving up.
const MAX_ALLOC_ATTEMPTS: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum LogError {
    NotFound,
    Read(String),
    Write(String),
    Parse(String),
    Serialize(String),
    Contended,
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::NotFound => write!(f, "conversation not found"),
            LogError::Read(e) => write!(f, "state read: {e}"),
            LogError::Write(e) => write!(f, "state write: {e}"),
            LogError::Parse(e) => write!(f, "state parse: {e}"),
            LogError::Serialize(e) => write!(f, "state serialize: {e}"),
            LogError::Contended => write!(f, "could not allocate an activity slot"),
        }
    }
}

pub struct ActivityPage {
    pub activities: Vec<StoredActivity>,
    /// Watermark the client should poll with next.
    pub watermark: u64,
}

pub fn activity_key(conv_key: &str, seq: u64) -> String {
    format!("{conv_key}/act/{seq:020}")
}

/// Write a header (no activities) for a conversation.
pub fn write_header<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    header: &ConversationState,
) -> Result<(), LogError> {
    let bytes = serde_json::to_vec(header).map_err(|e| LogError::Serialize(e.to_string()))?;
    store.write(conv_key, &bytes).map_err(LogError::Write)
}

/// Read the header (or legacy blob). `Ok(None)` when the conversation is absent.
pub fn read_header<S: StateStore>(
    store: &mut S,
    conv_key: &str,
) -> Result<Option<ConversationState>, LogError> {
    match store.read(conv_key).map_err(LogError::Read)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| LogError::Parse(e.to_string())),
        None => Ok(None),
    }
}

/// Copy a legacy blob's activities to their own keys, then rewrite the header.
fn migrate_legacy<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    header: &mut ConversationState,
) -> Result<(), LogError> {
    for activity in &header.activities {
        let bytes = serde_json::to_vec(activity).map_err(|e| LogError::Serialize(e.to_string()))?;
        let key = activity_key(conv_key, activity.watermark);
        // An existing slot is an earlier, interrupted migration of this activity.
        if store
            .write_if_absent(&key, &bytes)
            .map_err(LogError::Write)?
            .is_none()
        {
            store.write(&key, &bytes).map_err(LogError::Write)?;
        }
        header.next_watermark = header
            .next_watermark
            .max(activity.watermark.saturating_add(1));
    }
    header.activities.clear();
    header.layout = LAYOUT_PER_ACTIVITY;
    write_header(store, conv_key, header)
}

/// Append one activity, assigning it the next free watermark. `make` receives
/// the candidate watermark and may be called again on a retry.
pub fn append_activity<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    make: impl Fn(u64) -> StoredActivity,
) -> Result<StoredActivity, LogError> {
    let mut header = read_header(store, conv_key)?.ok_or(LogError::NotFound)?;
    if header.is_legacy() {
        migrate_legacy(store, conv_key, &mut header)?;
    }

    let mut seq = header.next_watermark;
    for _ in 0..MAX_ALLOC_ATTEMPTS {
        let mut activity = make(seq);
        activity.watermark = seq;
        let bytes =
            serde_json::to_vec(&activity).map_err(|e| LogError::Serialize(e.to_string()))?;
        let key = activity_key(conv_key, seq);
        match store
            .write_if_absent(&key, &bytes)
            .map_err(LogError::Write)?
        {
            Some(true) => {
                commit_header(store, conv_key, header, seq)?;
                return Ok(activity);
            }
            Some(false) => {
                seq = seq.saturating_add(1);
                continue;
            }
            None => {}
        }
        if store.read(&key).map_err(LogError::Read)?.is_some() {
            seq = seq.saturating_add(1);
            continue;
        }
        store.write(&key, &bytes).map_err(LogError::Write)?;
        let held = store.read(&key).map_err(LogError::Read)?;
        if held.as_deref() != Some(bytes.as_slice()) {
            seq = seq.saturating_add(1);
            continue;
        }
        commit_header(store, conv_key, header, seq)?;
        return Ok(activity);
    }
    Err(LogError::Contended)
}

/// Advance the header hint past `seq`, keeping whatever a concurrent writer
/// stored there meanwhile (flow binding) and never moving the hint backwards.
fn commit_header<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    ours: ConversationState,
    seq: u64,
) -> Result<(), LogError> {
    let mut header = read_header(store, conv_key)?.unwrap_or(ours);
    header.activities.clear();
    header.layout = LAYOUT_PER_ACTIVITY;
    header.next_watermark = header.next_watermark.max(seq.saturating_add(1));
    write_header(store, conv_key, &header)
}

/// Activities with `watermark >= since`, at most `page_size` slots per call.
pub fn read_activities<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    header: &ConversationState,
    since: Option<u64>,
) -> Result<ActivityPage, LogError> {
    read_activities_paged(store, conv_key, header, since, ACTIVITY_PAGE_SIZE)
}

pub fn read_activities_paged<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    header: &ConversationState,
    since: Option<u64>,
    page_size: usize,
) -> Result<ActivityPage, LogError> {
    let floor = since.unwrap_or(0);
    if header.is_legacy() {
        let activities = header
            .activities
            .iter()
            .filter(|a| a.watermark >= floor)
            .cloned()
            .collect();
        return Ok(ActivityPage {
            activities,
            watermark: header.next_watermark,
        });
    }

    let mut activities = Vec::new();
    let mut seq = floor;
    let mut reads = 0usize;
    loop {
        if reads >= page_size {
            return Ok(ActivityPage {
                activities,
                watermark: seq,
            });
        }
        match store
            .read(&activity_key(conv_key, seq))
            .map_err(LogError::Read)?
        {
            Some(bytes) => activities.push(
                serde_json::from_slice::<StoredActivity>(&bytes)
                    .map_err(|e| LogError::Parse(e.to_string()))?,
            ),
            // Past the hint the first empty slot is the tail; below it an
            // empty slot is a lost write and is skipped.
            None if seq >= header.next_watermark => break,
            None => {}
        }
        reads += 1;
        seq = seq.saturating_add(1);
    }
    Ok(ActivityPage {
        activities,
        watermark: header.next_watermark.max(seq),
    })
}

/// A copy of `header` carrying the activities with `watermark >= since` and a
/// `next_watermark` healed by probing past the header's hint. Callers that only
/// need the tail of a conversation (typing indicator) use this instead of
/// reading the whole history.
pub fn with_tail<S: StateStore>(
    store: &mut S,
    conv_key: &str,
    header: &ConversationState,
    since: u64,
) -> Result<ConversationState, LogError> {
    let page = read_activities(store, conv_key, header, Some(since))?;
    let mut tail = header.clone();
    tail.activities = page.activities;
    tail.next_watermark = header.next_watermark.max(page.watermark);
    Ok(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directline::jwt::DirectLineContext;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type AfterWrite = Box<dyn FnMut(&mut HashMap<String, Vec<u8>>, &str) + Send>;

    #[derive(Default)]
    struct MemStore {
        map: HashMap<String, Vec<u8>>,
        after_write: Option<AfterWrite>,
        claim: bool,
    }

    impl StateStore for MemStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.map.get(key).cloned())
        }
        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            self.map.insert(key.to_string(), value.to_vec());
            if let Some(hook) = self.after_write.as_mut() {
                hook(&mut self.map, key);
            }
            Ok(())
        }
        fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<Option<bool>, String> {
            if !self.claim {
                return Ok(None);
            }
            if self.map.contains_key(key) {
                return Ok(Some(false));
            }
            self.map.insert(key.to_string(), value.to_vec());
            Ok(Some(true))
        }
    }

    struct SharedStore(Arc<Mutex<MemStore>>);

    impl StateStore for SharedStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
            self.0.lock().unwrap().read(key)
        }
        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().write(key, value)
        }
        fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<Option<bool>, String> {
            self.0.lock().unwrap().write_if_absent(key, value)
        }
    }

    const KEY: &str = "webchat:conv:e:t:_:c1";

    fn ctx() -> DirectLineContext {
        DirectLineContext {
            env: "e".into(),
            tenant: "t".into(),
            team: None,
        }
    }

    fn act(id: &str, wm: u64) -> StoredActivity {
        StoredActivity {
            id: id.into(),
            type_: "message".into(),
            text: Some(format!("t{wm}")),
            from: Some("u".into()),
            timestamp: 1,
            watermark: wm,
            raw: json!({}),
        }
    }

    fn fresh() -> MemStore {
        let mut s = MemStore::default();
        write_header(&mut s, KEY, &ConversationState::new(ctx())).unwrap();
        s
    }

    fn claiming() -> MemStore {
        let mut s = fresh();
        s.claim = true;
        s
    }

    fn push(s: &mut MemStore, id: &str) -> StoredActivity {
        append_activity(s, KEY, |wm| act(id, wm)).unwrap()
    }

    #[test]
    fn key_layout_is_zero_padded_under_the_conversation_key() {
        assert_eq!(
            activity_key(KEY, 7),
            "webchat:conv:e:t:_:c1/act/00000000000000000007"
        );
        // Lexicographic order equals numeric order.
        assert!(activity_key(KEY, 9) < activity_key(KEY, 10));
    }

    #[test]
    fn append_writes_one_key_per_activity_and_keeps_header_small() {
        let mut s = fresh();
        for i in 0..3 {
            let a = push(&mut s, &format!("a{i}"));
            assert_eq!(a.watermark, i);
        }
        assert_eq!(s.map.len(), 4);
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        assert!(header.activities.is_empty());
        assert_eq!(header.next_watermark, 3);
        assert_eq!(header.layout, LAYOUT_PER_ACTIVITY);
    }

    #[test]
    fn read_returns_activities_in_order_and_honours_watermark() {
        let mut s = fresh();
        for i in 0..5 {
            push(&mut s, &format!("a{i}"));
        }
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let all = read_activities(&mut s, KEY, &header, None).unwrap();
        let ids: Vec<_> = all.activities.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a0", "a1", "a2", "a3", "a4"]);
        assert_eq!(all.watermark, 5);
        let tail = read_activities(&mut s, KEY, &header, Some(3)).unwrap();
        assert_eq!(tail.activities.len(), 2);
        let none = read_activities(&mut s, KEY, &header, Some(5)).unwrap();
        assert!(none.activities.is_empty());
        assert_eq!(none.watermark, 5);
    }

    #[test]
    fn pagination_continues_from_the_returned_watermark() {
        let mut s = fresh();
        for i in 0..7 {
            push(&mut s, &format!("a{i}"));
        }
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let mut since = None;
        let mut seen = Vec::new();
        for _ in 0..10 {
            let page = read_activities_paged(&mut s, KEY, &header, since, 3).unwrap();
            seen.extend(page.activities.iter().map(|a| a.watermark));
            if page.activities.is_empty() {
                break;
            }
            since = Some(page.watermark);
        }
        assert_eq!(seen, (0..7).collect::<Vec<_>>());
    }

    #[test]
    fn legacy_blob_is_readable_unchanged() {
        let mut s = MemStore::default();
        let mut legacy = ConversationState::new(ctx());
        legacy.layout = 0;
        legacy.activities = vec![act("old0", 0), act("old1", 1)];
        legacy.next_watermark = 2;
        s.map
            .insert(KEY.into(), serde_json::to_vec(&legacy).unwrap());
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        assert!(header.is_legacy());
        let page = read_activities(&mut s, KEY, &header, Some(1)).unwrap();
        assert_eq!(page.activities.len(), 1);
        assert_eq!(page.watermark, 2);
    }

    #[test]
    fn legacy_blob_migrates_on_next_append_without_losing_history() {
        let mut s = MemStore::default();
        let mut legacy = ConversationState::new(ctx());
        legacy.layout = 0;
        legacy.flow_binding = Some("flow".into());
        legacy.activities = vec![act("old0", 0), act("old1", 1)];
        legacy.next_watermark = 2;
        s.map
            .insert(KEY.into(), serde_json::to_vec(&legacy).unwrap());

        let added = push(&mut s, "new");
        assert_eq!(added.watermark, 2);

        let header = read_header(&mut s, KEY).unwrap().unwrap();
        assert!(!header.is_legacy());
        assert!(header.activities.is_empty());
        assert_eq!(header.flow_binding.as_deref(), Some("flow"));
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        let ids: Vec<_> = page.activities.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["old0", "old1", "new"]);
    }

    #[test]
    fn stale_header_hint_never_overwrites_an_existing_activity() {
        let mut s = fresh();
        push(&mut s, "a0");
        push(&mut s, "a1");
        // Regress the hint, as a lost header write would.
        let mut header = read_header(&mut s, KEY).unwrap().unwrap();
        header.next_watermark = 0;
        write_header(&mut s, KEY, &header).unwrap();

        let a = push(&mut s, "a2");
        assert_eq!(a.watermark, 2);
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        assert_eq!(header.next_watermark, 3);
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        assert_eq!(page.activities.len(), 3);
    }

    #[test]
    fn reader_probes_past_a_stale_hint() {
        let mut s = fresh();
        push(&mut s, "a0");
        push(&mut s, "a1");
        let mut header = read_header(&mut s, KEY).unwrap().unwrap();
        header.next_watermark = 1;
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        assert_eq!(page.activities.len(), 2);
        assert_eq!(page.watermark, 2);
    }

    #[test]
    fn overwritten_slot_is_detected_and_the_append_moves_on() {
        let mut s = fresh();
        let target = activity_key(KEY, 0);
        // A concurrent writer replaces slot 0 right after our write.
        s.after_write = Some(Box::new(move |map, key| {
            if key == target {
                let other = serde_json::to_vec(&act("other", 0)).unwrap();
                map.insert(key.to_string(), other);
            }
        }));
        let ours = push(&mut s, "ours");
        assert_eq!(ours.watermark, 1);
        s.after_write = None;
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        let ids: Vec<_> = page.activities.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["other", "ours"]);
    }

    #[test]
    fn concurrent_appends_each_claim_a_distinct_slot() {
        const THREADS: usize = 4;
        const PER_THREAD: usize = 8;
        let shared = Arc::new(Mutex::new(claiming()));
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let mut store = SharedStore(Arc::clone(&shared));
                std::thread::spawn(move || {
                    (0..PER_THREAD)
                        .map(|i| {
                            append_activity(&mut store, KEY, |wm| act(&format!("t{t}-{i}"), wm))
                                .unwrap()
                                .watermark
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut watermarks: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        watermarks.sort_unstable();
        let total = (THREADS * PER_THREAD) as u64;
        assert_eq!(watermarks, (0..total).collect::<Vec<_>>());

        let mut s = SharedStore(shared);
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        let mut ids: Vec<_> = page.activities.iter().map(|a| a.id.clone()).collect();
        ids.sort();
        let mut expected: Vec<_> = (0..THREADS)
            .flat_map(|t| (0..PER_THREAD).map(move |i| format!("t{t}-{i}")))
            .collect();
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn append_without_conditional_write_still_assigns_contiguous_watermarks() {
        let mut s = fresh();
        assert!(!s.claim);
        let marks: Vec<_> = (0..5)
            .map(|i| push(&mut s, &format!("a{i}")).watermark)
            .collect();
        assert_eq!(marks, [0, 1, 2, 3, 4]);
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        assert_eq!(page.activities.len(), 5);
    }

    #[test]
    fn a_claimed_slot_is_never_overwritten_and_the_append_moves_on() {
        let mut s = claiming();
        let existing = serde_json::to_vec(&act("existing", 0)).unwrap();
        s.map.insert(activity_key(KEY, 0), existing.clone());

        let ours = push(&mut s, "ours");
        assert_eq!(ours.watermark, 1);
        assert_eq!(s.map.get(&activity_key(KEY, 0)), Some(&existing));
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        assert_eq!(header.next_watermark, 2);
    }

    #[test]
    fn repeating_a_legacy_migration_loses_nothing() {
        let mut s = MemStore {
            claim: true,
            ..MemStore::default()
        };
        let mut legacy = ConversationState::new(ctx());
        legacy.layout = 0;
        legacy.activities = vec![act("old0", 0), act("old1", 1)];
        legacy.next_watermark = 2;
        let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
        s.map.insert(KEY.into(), legacy_bytes.clone());

        let mut header = read_header(&mut s, KEY).unwrap().unwrap();
        migrate_legacy(&mut s, KEY, &mut header).unwrap();
        let migrated = s.map.get(&activity_key(KEY, 1)).cloned();
        // A crash before the header rewrite leaves the legacy blob in place.
        s.map.insert(KEY.into(), legacy_bytes);

        let added = push(&mut s, "new");
        assert_eq!(added.watermark, 2);
        assert_eq!(s.map.get(&activity_key(KEY, 1)).cloned(), migrated);
        let header = read_header(&mut s, KEY).unwrap().unwrap();
        let page = read_activities(&mut s, KEY, &header, None).unwrap();
        let ids: Vec<_> = page.activities.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["old0", "old1", "new"]);
    }

    #[test]
    fn append_to_a_missing_conversation_is_not_found() {
        let mut s = MemStore::default();
        let r = append_activity(&mut s, KEY, |wm| act("x", wm));
        assert_eq!(r.unwrap_err(), LogError::NotFound);
    }
}
