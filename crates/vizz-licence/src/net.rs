//! The four calls to the licence service, behind a seam tests can stub.
//!
//! Blocking, deliberately: vizz already talks HTTP through `ureq` on a
//! background thread for the update check, and every call here is made
//! the same way — from a thread [`crate::Licence`] spawns, never from the
//! render or UI thread, never on the startup path.
//!
//! What each call guarantees before it hands a token back:
//!
//! - `machine` was sent RAW, and `product` was sent on activate and
//!   heartbeat (the service refuses another product's key before it takes
//!   a seat when it is told which product is asking).
//! - The service's echo of the machine equals our own hash of what we
//!   sent. If it does not, the token is for somebody else's idea of this
//!   machine and is refused here — storing it would be the silent
//!   `wrong_machine` dead end.
//! - A refusal carries the service's own `message`, which is what the
//!   licence section shows.

use std::time::Duration;

use serde_json::{Value, json};

use crate::verify::machine_hash;

/// Short enough that a dead venue network gives up well before anyone
/// wonders what the button is doing.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The HTTP boundary: POST a JSON body, get the status and the body back.
/// An `Err` means nothing came back at all — no network, DNS, TLS, a
/// timeout — and is the one case that keeps the cached answer silently.
pub trait Transport: Send + Sync {
    fn post(&self, url: &str, body: &str) -> Result<(u16, String), String>;
}

/// The real one, over `ureq` and rustls — the stack vizz-update uses.
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            // The service answers refusals with a JSON body on a 4xx, and
            // that body is the message to show. ureq would otherwise turn
            // the status into an error and throw the body away.
            .http_status_as_error(false)
            .build()
            .new_agent();
        Http { agent }
    }
}

impl Transport for Http {
    fn post(&self, url: &str, body: &str) -> Result<(u16, String), String> {
        let mut res = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("User-Agent", concat!("vizz/", env!("CARGO_PKG_VERSION")))
            .send(body)
            .map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
        Ok((status, text))
    }
}

/// Why a call did not produce what was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Nothing came back. Keep the cached answer; say nothing alarming.
    Offline(String),
    /// The service said no, in its own words: `{ok:false, reason, message}`.
    Refused { reason: String, message: String },
    /// Something came back that is not what the service sends, or a token
    /// that must not be stored. Nothing was stored.
    Protocol(String),
}

impl Failure {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Failure::Refused { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Offline(e) => write!(f, "could not reach the licence service ({e})"),
            // The service's sentence, verbatim: it is written for people.
            Failure::Refused { message, .. } => f.write_str(message),
            Failure::Protocol(e) => f.write_str(e),
        }
    }
}

/// What activate, heartbeat and trial return: a token for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    pub token: String,
    /// Only the trial call returns one — it is how a trial gets a key to
    /// check in with later.
    pub key: Option<String>,
}

/// Everything a call needs to say who is asking.
pub struct Caller<'a> {
    pub transport: &'a dyn Transport,
    pub base: &'a str,
    pub product: &'a str,
    /// RAW. See the module notes.
    pub fingerprint: &'a str,
}

impl Caller<'_> {
    pub fn activate(&self, key: &str, label: &str) -> Result<Issued, Failure> {
        let mut body = json!({
            "key": key,
            "machine": self.fingerprint,
            "product": self.product,
        });
        // What the account page calls this seat. Optional on the wire, so
        // left out rather than sent empty.
        if !label.trim().is_empty() {
            body["label"] = json!(label.trim());
        }
        self.issued(self.call("/api/licence/activate", &body)?)
    }

    pub fn heartbeat(&self, key: &str) -> Result<Issued, Failure> {
        let body = json!({ "key": key, "machine": self.fingerprint, "product": self.product });
        self.issued(self.call("/api/licence/heartbeat", &body)?)
    }

    pub fn trial(&self, email: &str, name: &str) -> Result<Issued, Failure> {
        let mut body = json!({
            "product": self.product,
            "email": email,
            "machine": self.fingerprint,
        });
        if !name.is_empty() {
            body["name"] = json!(name);
        }
        self.issued(self.call("/api/licence/trial", &body)?)
    }

    pub fn deactivate(&self, key: &str) -> Result<(), Failure> {
        let body = json!({ "key": key, "machine": self.fingerprint });
        self.call("/api/licence/deactivate", &body).map(|_| ())
    }

    fn call(&self, path: &str, body: &Value) -> Result<Value, Failure> {
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        let (status, text) = self.transport.post(&url, &body.to_string()).map_err(Failure::Offline)?;
        let reply: Value = serde_json::from_str(&text).map_err(|_| {
            // A proxy's HTML error page, a captive portal, a 502 from a
            // load balancer: not a refusal, and not worth quoting.
            Failure::Protocol(format!("the licence service sent back something unreadable (HTTP {status})"))
        })?;
        if reply["ok"] == Value::Bool(true) && (200..300).contains(&status) {
            return Ok(reply);
        }
        let reason = reply["reason"].as_str().unwrap_or("server_error").to_string();
        let message = reply["message"]
            .as_str()
            .filter(|m| !m.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("the licence service refused the request (HTTP {status})"));
        Err(Failure::Refused { reason, message })
    }

    /// A reply that claims to carry a token, checked before anything is
    /// stored.
    fn issued(&self, reply: Value) -> Result<Issued, Failure> {
        let token = reply["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Failure::Protocol("the licence service returned no token".into()))?;
        let ours = machine_hash(self.fingerprint);
        match reply["machine"].as_str() {
            Some(theirs) if theirs == ours => {}
            theirs => {
                return Err(Failure::Protocol(format!(
                    "the licence service recorded this machine as {} but it is {ours} — \
                     nothing was stored",
                    theirs.unwrap_or("nothing")
                )));
            }
        }
        Ok(Issued { token: token.to_string(), key: reply["key"].as_str().map(str::to_string) })
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use std::sync::Mutex;

    use super::Transport;

    /// Records every request and answers from a queue of canned replies.
    /// An `Err` in the queue is a network failure.
    #[derive(Default)]
    pub struct Stub {
        pub sent: Mutex<Vec<(String, serde_json::Value)>>,
        pub replies: Mutex<Vec<Result<(u16, String), String>>>,
    }

    impl Stub {
        pub fn answering(replies: Vec<Result<(u16, String), String>>) -> Self {
            Stub { sent: Mutex::default(), replies: Mutex::new(replies) }
        }
        pub fn sent(&self) -> Vec<(String, serde_json::Value)> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl Transport for Stub {
        fn post(&self, url: &str, body: &str) -> Result<(u16, String), String> {
            let parsed = serde_json::from_str(body).expect("the client sends JSON");
            self.sent.lock().unwrap().push((url.to_string(), parsed));
            let mut replies = self.replies.lock().unwrap();
            if replies.is_empty() {
                return Err("no reply queued".into());
            }
            replies.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stub::Stub;
    use super::*;

    const FP: &str = "ABC123-MACHINE-SERIAL";
    const HASH: &str = "8b9dd6da2bcf47bdfe7ceb27c2a58680";

    fn caller(t: &Stub) -> Caller<'_> {
        Caller { transport: t, base: "https://letissier.ie", product: "vizz", fingerprint: FP }
    }

    /// Shaped exactly like the service's documented activate reply.
    fn activate_reply(machine: &str) -> String {
        json!({
            "ok": true,
            "token": "eyJ2IjoxfQ.c2ln",
            "machine": machine,
            "product": "vizz",
            "edition": "standard",
            "seats": 2,
            "seatsUsed": 1,
            "checkInBy": "2026-09-18T00:00:00.000Z",
            "maintenanceUntil": "2027-08-21T00:00:00.000Z"
        })
        .to_string()
    }

    fn refusal(reason: &str, message: &str) -> String {
        json!({ "ok": false, "reason": reason, "message": message }).to_string()
    }

    #[test]
    fn activate_sends_the_raw_fingerprint_and_the_product() {
        let t = Stub::answering(vec![Ok((200, activate_reply(HASH)))]);
        let issued = caller(&t).activate("LT-V1ZZ-K7M2-9PQR-4XTC", "FOH laptop").unwrap();
        assert_eq!(issued.token, "eyJ2IjoxfQ.c2ln");
        let sent = t.sent();
        assert_eq!(sent.len(), 1);
        let (url, body) = &sent[0];
        assert_eq!(url, "https://letissier.ie/api/licence/activate");
        assert_eq!(body["machine"], FP, "the machine must go on the wire RAW");
        assert_ne!(body["machine"], HASH, "pre-hashing mints a token for a machine that does not exist");
        assert_eq!(body["product"], "vizz");
        assert_eq!(body["key"], "LT-V1ZZ-K7M2-9PQR-4XTC");
        assert_eq!(body["label"], "FOH laptop");

        // No label: left out rather than sent empty.
        let t = Stub::answering(vec![Ok((200, activate_reply(HASH)))]);
        caller(&t).activate("LT-V1ZZ-K7M2-9PQR-4XTC", " ").unwrap();
        assert!(t.sent()[0].1.get("label").is_none());
    }

    /// The double-hash bug, as the service would reply to a client that
    /// had it: the echo is the hash of our hash. Refused, nothing issued.
    #[test]
    fn an_echoed_machine_that_is_not_ours_is_refused() {
        let hash_of_hash = machine_hash(HASH);
        let t = Stub::answering(vec![Ok((200, activate_reply(&hash_of_hash)))]);
        let err = caller(&t).activate("LT-V1ZZ-K7M2-9PQR-4XTC", "").unwrap_err();
        assert!(
            matches!(&err, Failure::Protocol(m) if m.contains(&hash_of_hash) && m.contains(HASH)),
            "{err:?}"
        );

        // No echo at all is no better.
        let mut reply: Value = serde_json::from_str(&activate_reply(HASH)).unwrap();
        reply.as_object_mut().unwrap().remove("machine");
        let t = Stub::answering(vec![Ok((200, reply.to_string()))]);
        assert!(matches!(caller(&t).activate("k", ""), Err(Failure::Protocol(_))));
    }

    #[test]
    fn heartbeat_sends_the_product_and_the_raw_fingerprint() {
        let reply = json!({
            "ok": true, "token": "new.token", "machine": HASH,
            "checkInBy": "2026-10-18T00:00:00.000Z", "maintenanceUntil": "2027-08-21T00:00:00.000Z"
        });
        let t = Stub::answering(vec![Ok((200, reply.to_string()))]);
        let issued = caller(&t).heartbeat("LT-V1ZZ-K7M2-9PQR-4XTC").unwrap();
        assert_eq!(issued.token, "new.token");
        let (url, body) = &t.sent()[0];
        assert!(url.ends_with("/api/licence/heartbeat"));
        assert_eq!(body["machine"], FP);
        assert_eq!(body["product"], "vizz");
    }

    #[test]
    fn a_trial_returns_its_key_and_sends_the_email() {
        let reply = json!({
            "ok": true, "key": "LT-V1ZZ-TR1A-L000-0000", "token": "trial.token",
            "machine": HASH, "expiresAt": "2026-10-24T00:00:00.000Z",
            "checkInBy": "2026-10-24T00:00:00.000Z"
        });
        let t = Stub::answering(vec![Ok((200, reply.to_string()))]);
        let issued = caller(&t).trial("vj@example.com", "VJ").unwrap();
        assert_eq!(issued.key.as_deref(), Some("LT-V1ZZ-TR1A-L000-0000"));
        let (url, body) = &t.sent()[0];
        assert!(url.ends_with("/api/licence/trial"));
        assert_eq!(body["product"], "vizz");
        assert_eq!(body["email"], "vj@example.com");
        assert_eq!(body["name"], "VJ");
        assert_eq!(body["machine"], FP);

        // No name typed: the field is left out rather than sent empty.
        let t = Stub::answering(vec![Ok((200, reply.to_string()))]);
        caller(&t).trial("vj@example.com", "").unwrap();
        assert!(t.sent()[0].1.get("name").is_none());
    }

    /// Every documented refusal comes through with the service's own
    /// sentence, which is what the panel shows.
    #[test]
    fn refusals_carry_the_services_message() {
        for (status, reason) in [
            (400, "bad_request"),
            (400, "malformed_key"),
            (400, "bad_email"),
            (404, "unknown_key"),
            (404, "not_activated"),
            (403, "revoked"),
            (403, "expired"),
            (409, "no_seats"),
            (409, "trial_already_used"),
            (409, "wrong_product"),
            (500, "server_error"),
        ] {
            let message = format!("the service explains {reason}");
            let t = Stub::answering(vec![Ok((status, refusal(reason, &message)))]);
            let err = caller(&t).activate("k", "").unwrap_err();
            assert_eq!(err.reason(), Some(reason));
            assert_eq!(err.to_string(), message, "the message must be shown verbatim");
        }
    }

    #[test]
    fn a_wrong_product_refusal_names_the_other_product() {
        let msg = "That key is for Light. It cannot activate Vizz.";
        let t = Stub::answering(vec![Ok((409, refusal("wrong_product", msg)))]);
        assert_eq!(caller(&t).activate("LT-11GH-AAAA-BBBB-CCCC", "").unwrap_err().to_string(), msg);
    }

    #[test]
    fn no_network_and_garbage_are_told_apart_from_a_refusal() {
        let t = Stub::answering(vec![Err("dns error".into())]);
        assert!(matches!(caller(&t).heartbeat("k"), Err(Failure::Offline(_))));
        let t = Stub::answering(vec![Ok((502, "<html>Bad Gateway</html>".into()))]);
        assert!(matches!(caller(&t).heartbeat("k"), Err(Failure::Protocol(_))));
        // A 2xx that says ok:false is still a refusal.
        let t = Stub::answering(vec![Ok((200, refusal("revoked", "Refunded.")))]);
        assert_eq!(caller(&t).heartbeat("k").unwrap_err().reason(), Some("revoked"));
    }

    #[test]
    fn deactivate_sends_key_and_raw_machine() {
        let t = Stub::answering(vec![Ok((200, json!({ "ok": true }).to_string()))]);
        caller(&t).deactivate("LT-V1ZZ-K7M2-9PQR-4XTC").unwrap();
        let (url, body) = &t.sent()[0];
        assert!(url.ends_with("/api/licence/deactivate"));
        assert_eq!(body["machine"], FP);
        assert_eq!(body["key"], "LT-V1ZZ-K7M2-9PQR-4XTC");
    }
}
