//! Sending what the outbox holds.
//!
//! Only the outbox, only from a background thread, eight seconds at most
//! per request, and never a word on screen when it fails: a report that
//! does not go stays queued for the next launch that has a network. The
//! contract's answers decide what happens to each file:
//!
//! | answer | the file |
//! | --- | --- |
//! | 2xx | removed: the service has it |
//! | 400, 413 | removed: it will never be accepted, and retrying is noise |
//! | 429, anything else, no answer | kept for next launch |

use std::time::Duration;

use crate::queue::{Envelope, Queue};

/// The contract's timeout.
pub const TIMEOUT: Duration = Duration::from_secs(8);

/// POST a JSON body; the status and body back, or `Err` when nothing came
/// back at all.
pub trait Transport: Send + Sync {
    fn post(&self, url: &str, body: &str) -> Result<u16, String>;
}

/// The real one, over the same `ureq` + rustls stack as the update check
/// and the licence client.
pub struct Http {
    agent: ureq::Agent,
    user_agent: String,
}

impl Http {
    pub fn new(user_agent: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            // 4xx answers are verdicts to act on, not transport errors.
            .http_status_as_error(false)
            .build()
            .new_agent();
        Http { agent, user_agent }
    }
}

impl Transport for Http {
    fn post(&self, url: &str, body: &str) -> Result<u16, String> {
        let res = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("User-Agent", &self.user_agent)
            .send(body)
            .map_err(|e| e.to_string())?;
        Ok(res.status().as_u16())
    }
}

/// What one pass over the outbox did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flushed {
    pub sent: usize,
    pub dropped: usize,
    pub kept: usize,
}

/// The fate of a report, from the service's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    Sent,
    Drop,
    Keep,
}

pub fn fate(answer: &Result<u16, String>) -> Fate {
    match answer {
        Ok(s) if (200..300).contains(s) => Fate::Sent,
        Ok(400) | Ok(413) => Fate::Drop,
        _ => Fate::Keep,
    }
}

/// Offer every report in `outbox` to `base`. Stops at the first answer
/// that means "not now" (no network, rate limited, the service down),
/// since the rest would get the same answer eight seconds at a time.
pub fn flush(outbox: &Queue, transport: &dyn Transport, base: &str) -> Flushed {
    let mut done = Flushed::default();
    let entries = outbox.entries();
    let total = entries.len();
    for entry in entries {
        let body = match &entry.envelope {
            Envelope::Crash(c) => crate::payload::to_body(c),
            Envelope::Feedback(f) => serde_json::to_string(f).unwrap_or_default(),
        };
        let url = format!("{}{}", base.trim_end_matches('/'), entry.envelope.path());
        let answer = transport.post(&url, &body);
        match fate(&answer) {
            Fate::Sent => {
                let _ = std::fs::remove_file(&entry.path);
                done.sent += 1;
            }
            Fate::Drop => {
                log::warn!("the report service refused a queued report ({answer:?}) — dropped");
                let _ = std::fs::remove_file(&entry.path);
                done.dropped += 1;
            }
            Fate::Keep => {
                log::debug!("report not sent ({answer:?}) — kept for the next launch");
                done.kept = total - done.sent - done.dropped;
                return done;
            }
        }
    }
    done
}

#[cfg(test)]
pub(crate) mod stub {
    use std::sync::Mutex;

    use super::Transport;

    /// Records every request and answers from a queue of canned replies.
    #[derive(Default)]
    pub struct Stub {
        pub sent: Mutex<Vec<(String, serde_json::Value)>>,
        pub replies: Mutex<Vec<Result<u16, String>>>,
    }

    impl Stub {
        pub fn answering(replies: Vec<Result<u16, String>>) -> Self {
            Stub { sent: Mutex::default(), replies: Mutex::new(replies) }
        }
        pub fn sent(&self) -> Vec<(String, serde_json::Value)> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl Transport for Stub {
        fn post(&self, url: &str, body: &str) -> Result<u16, String> {
            let parsed = serde_json::from_str(body).expect("reports are JSON");
            self.sent.lock().unwrap().push((url.to_string(), parsed));
            let mut replies = self.replies.lock().unwrap();
            if replies.is_empty() {
                return Err("offline".into());
            }
            replies.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stub::Stub;
    use super::*;
    use crate::queue::tests::{crash, scratch};

    #[test]
    fn answers_decide_what_stays() {
        assert_eq!(fate(&Ok(202)), Fate::Sent);
        assert_eq!(fate(&Ok(400)), Fate::Drop);
        assert_eq!(fate(&Ok(413)), Fate::Drop);
        assert_eq!(fate(&Ok(429)), Fate::Keep);
        assert_eq!(fate(&Ok(500)), Fate::Keep);
        assert_eq!(fate(&Ok(404)), Fate::Keep, "a service not deployed yet must not eat the queue");
        assert_eq!(fate(&Err("timed out".into())), Fate::Keep);
    }

    #[test]
    fn a_flush_sends_in_order_and_stops_at_not_now() {
        let dir = scratch("flush");
        let q = Queue::new(&dir);
        for s in ["a", "b", "c", "d"] {
            q.push(&crash(s)).unwrap();
        }
        let t = Stub::answering(vec![Ok(202), Ok(400), Ok(429)]);
        let done = flush(&q, &t, "https://letissier.ie/");
        assert_eq!(done, Flushed { sent: 1, dropped: 1, kept: 2 });
        let sent = t.sent();
        assert_eq!(sent.len(), 3, "nothing after a 429 should have been tried");
        assert_eq!(sent[0].0, "https://letissier.ie/api/reports/crash");
        assert_eq!(sent[0].1["summary"], "a");
        assert_eq!(sent[2].1["summary"], "c");
        // c (rate limited) and d (never tried) wait for next launch.
        assert_eq!(q.len(), 2);

        // Next launch, network back.
        let t = Stub::answering(vec![Ok(202), Ok(202)]);
        assert_eq!(flush(&q, &t, "https://letissier.ie").sent, 2);
        assert!(q.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
