//! The vendor's Rust licence SDK, on the crypto this build already carries.
//!
//! A line-for-line port of `clients/rust/src/lib.rs` from the letissier.ie
//! repository ("LeTissier licence verification — Rust. Used by Vizz and
//! Light."), with one substitution: `ring` does the Ed25519 check and the
//! SHA-256 instead of `ed25519-dalek`, `sha2` and `hex`. Everything else —
//! the token shape, the claim fields and which are required, the order of
//! the checks, the `<= 0` lease boundary — is the SDK's, and the tests
//! drive it through the vendor's own published vectors to prove the two
//! agree.
//!
//! Pure: no clock, no disk, no network. Every function here is a function
//! of its arguments.
//!
//! One addition on top, [`check_for`], because the SDK leaves it to the
//! app: a token must be for *this* product. The vendor sells several and
//! signs them all with the same key, so without it a Datamosh licence
//! would license Vizz.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Claims {
    pub v: u8,
    pub key: String,
    pub product: String,
    pub edition: String,
    pub customer: String,
    #[serde(default)]
    pub name: Option<String>,
    pub seats: u32,
    /// Entitled to builds released at or before this unix time.
    #[serde(rename = "maintUntil")]
    pub maint_until: i64,
    /// Check-in deadline for this lease. For a trial it is also the end
    /// of the trial.
    pub exp: i64,
    pub machine: String,
    pub mode: String,
    pub iat: i64,
    pub jti: String,
}

impl Claims {
    pub fn is_trial(&self) -> bool {
        self.edition == "trial"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Good to run.
    Active,
    /// Licence is fine, but this build is newer than the update entitlement.
    UpdateRequired,
    /// Lease lapsed. Check in to renew; app policy decides any grace.
    CheckInRequired,
    /// A trial that has run out.
    Expired,
    /// Token was issued for a different machine.
    WrongMachine,
    /// Signature failed, malformed, wrong version — or, through
    /// [`check_for`], another product's licence.
    Invalid,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Active,
        Status::UpdateRequired,
        Status::CheckInRequired,
        Status::Expired,
        Status::WrongMachine,
        Status::Invalid,
    ];

    /// Stable string, matching the other language SDKs.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::UpdateRequired => "update_required",
            Status::CheckInRequired => "check_in_required",
            Status::Expired => "expired",
            Status::WrongMachine => "wrong_machine",
            Status::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub status: Status,
    /// Present only when the signature verified — never trust these
    /// otherwise.
    pub claims: Option<Claims>,
    /// Seconds until check-in is due; negative once overdue.
    pub check_in_in: Option<i64>,
}

impl Verdict {
    pub fn invalid() -> Self {
        Verdict { status: Status::Invalid, claims: None, check_in_in: None }
    }
}

/// Must match the server exactly: sha256 of the trimmed fingerprint,
/// first 32 hex characters.
///
/// For comparing against a token's claim and for showing a short machine
/// id. Never for the wire: the service hashes what it is sent, so sending
/// this mints a token for a machine that does not exist.
pub fn machine_hash(fingerprint: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, fingerprint.trim().as_bytes());
    let mut hex = to_hex(digest.as_ref());
    hex.truncate(32);
    hex
}

/// Verify the signature and parse the claims. No clock or machine checks.
///
/// The signature is over the ASCII of the base64url payload *segment*, not
/// the decoded bytes — which is what the SDK does and what the vectors
/// only verify against.
pub fn verify(token: &str, public_key_hex: &str) -> Option<Claims> {
    let (payload, signature) = token.split_once('.')?;

    let key_bytes: [u8; 32] = from_hex(public_key_hex)?.try_into().ok()?;
    let signature_bytes: [u8; 64] = URL_SAFE_NO_PAD.decode(signature).ok()?.try_into().ok()?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key_bytes)
        .verify(payload.as_bytes(), &signature_bytes)
        .ok()?;

    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;

    if claims.v == 1 { Some(claims) } else { None }
}

/// The whole decision, offline — the SDK's `check`, unchanged.
///
/// `build_date` is when THIS build was released (unix seconds), baked in
/// at compile time. An older build stays entitled forever; a newer one
/// asks for a renewal.
///
/// `fingerprint` is the RAW machine id, hashed here, exactly as the SDK
/// takes it.
pub fn check(token: &str, fingerprint: &str, build_date: i64, now: i64, public_key_hex: &str) -> Verdict {
    let Some(claims) = verify(token, public_key_hex) else {
        return Verdict::invalid();
    };

    if claims.machine != machine_hash(fingerprint) {
        return Verdict { status: Status::WrongMachine, claims: Some(claims), check_in_in: None };
    }

    let check_in_in = claims.exp - now;

    if check_in_in <= 0 {
        // A trial's lease is its lifetime, so a lapsed trial is simply over.
        let status = if claims.is_trial() { Status::Expired } else { Status::CheckInRequired };
        return Verdict { status, claims: Some(claims), check_in_in: Some(check_in_in) };
    }

    if build_date > claims.maint_until {
        return Verdict {
            status: Status::UpdateRequired,
            claims: Some(claims),
            check_in_in: Some(check_in_in),
        };
    }

    Verdict { status: Status::Active, claims: Some(claims), check_in_in: Some(check_in_in) }
}

/// [`check`], and the token must be for `product`.
///
/// Another product's token is `Invalid` with no claims: it is not a
/// licence for this app in any sense, and handing its claims on would
/// invite something downstream to show "Licensed to …" for it.
///
/// `product` is a parameter rather than the crate's constant so the
/// vendor's vectors, which are all issued for `vizz`, can prove the
/// rejection by asking for something else.
pub fn check_for(
    token: &str,
    fingerprint: &str,
    build_date: i64,
    now: i64,
    public_key_hex: &str,
    product: &str,
) -> Verdict {
    let verdict = check(token, fingerprint, build_date, now, public_key_hex);
    match &verdict.claims {
        Some(claims) if claims.product != product => Verdict::invalid(),
        _ => verdict,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Strict: an even number of hex digits and nothing else, as `hex::decode`
/// would have it.
pub(crate) fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The vendor's published vectors, verbatim from
    /// `clients/vectors.json` ("Every SDK must agree with these"). Signed
    /// with a TEST key carried in the file, not the production key.
    const VECTORS: &str = include_str!("../testdata/vectors.json");

    struct Vectors(Value);

    impl Vectors {
        fn load() -> Self {
            Vectors(serde_json::from_str(VECTORS).expect("vectors.json parses"))
        }
        fn str(&self, path: &[&str]) -> String {
            let mut v = &self.0;
            for p in path {
                v = &v[*p];
            }
            v.as_str().unwrap_or_else(|| panic!("{path:?} is not a string")).to_string()
        }
        fn int(&self, path: &[&str]) -> i64 {
            let mut v = &self.0;
            for p in path {
                v = &v[*p];
            }
            v.as_i64().unwrap_or_else(|| panic!("{path:?} is not an integer"))
        }
        fn key(&self) -> String {
            self.str(&["publicKeyHex"])
        }
        fn token(&self, name: &str) -> String {
            self.str(&["tokens", name])
        }
        fn fingerprint(&self) -> String {
            self.str(&["fingerprint"])
        }
    }

    fn status(s: &str) -> Status {
        *Status::ALL
            .iter()
            .find(|st| st.as_str() == s)
            .unwrap_or_else(|| panic!("the vectors name a status the SDK does not have: {s}"))
    }

    /// The same table the vendor's `examples/vectors.rs` harness prints,
    /// asserted instead of printed.
    #[test]
    fn the_published_vectors_verify_as_documented() {
        let v = Vectors::load();
        let key = v.key();
        assert_eq!(machine_hash(&v.fingerprint()), v.str(&["machineHash"]));
        assert!(verify(&v.token("valid"), &key).is_some(), "valid");
        assert!(verify(&v.token("validTrial"), &key).is_some(), "validTrial");
        assert!(verify(&v.token("tampered"), &key).is_none(), "tampered: seats 2 -> 99");
        assert!(verify(&v.token("wrongKey"), &key).is_none(), "signed by an untrusted key");
        assert!(verify(&v.token("malformed"), &key).is_none(), "malformed");

        let claims = verify(&v.token("valid"), &key).unwrap();
        assert_eq!(claims.product, v.str(&["claims", "product"]));
        assert_eq!(claims.seats as i64, v.int(&["claims", "seats"]));
        assert_eq!(claims.maint_until, v.int(&["claims", "maintUntil"]));
        assert_eq!(claims.exp, v.int(&["claims", "exp"]));
        assert_eq!(claims.name.as_deref(), Some(v.str(&["claims", "name"]).as_str()));
    }

    /// Every status case in the file, driven from the file — a case added
    /// to the vectors is a case checked here without touching this test.
    #[test]
    fn every_status_case_in_the_vectors_agrees() {
        let v = Vectors::load();
        let key = v.key();
        let fp = v.fingerprint();
        let now = v.int(&["now"]);
        let iat = v.int(&["claims", "iat"]);
        let mut checked = 0;
        for case in v.0["entitlement"].as_array().unwrap() {
            let build = case["buildDate"].as_i64().unwrap();
            let got = check(&v.token("valid"), &fp, build, now, &key).status;
            assert_eq!(got, status(case["expect"].as_str().unwrap()), "{}", case["why"]);
            checked += 1;
        }
        for case in v.0["lease"].as_array().unwrap() {
            let at = case["at"].as_i64().unwrap();
            let got = check(&v.token("valid"), &fp, iat, at, &key).status;
            assert_eq!(got, status(case["expect"].as_str().unwrap()), "{}", case["why"]);
            checked += 1;
        }
        for case in v.0["trialLease"].as_array().unwrap() {
            let at = case["at"].as_i64().unwrap();
            let got = check(&v.token("validTrial"), &fp, iat, at, &key).status;
            assert_eq!(got, status(case["expect"].as_str().unwrap()), "{}", case["why"]);
            checked += 1;
        }
        assert_eq!(checked, 6, "the vectors shrank; this test would pass on nothing");

        // And the two the harness prints outside the tables.
        assert_eq!(
            check(&v.token("valid"), "SOME-OTHER-MACHINE", iat, now, &key).status,
            Status::WrongMachine
        );
        assert_eq!(check(&v.token("tampered"), &fp, iat, now, &key).status, Status::Invalid);
    }

    /// The lease boundary the vectors do not cover, pinned to the SDK's
    /// `<= 0`: at the exact second of `exp` the lease has lapsed.
    #[test]
    fn the_lease_lapses_at_exp_not_after_it() {
        let v = Vectors::load();
        let (key, fp) = (v.key(), v.fingerprint());
        let exp = v.int(&["claims", "exp"]);
        let iat = v.int(&["claims", "iat"]);
        assert_eq!(check(&v.token("valid"), &fp, iat, exp - 1, &key).status, Status::Active);
        assert_eq!(check(&v.token("valid"), &fp, iat, exp, &key).status, Status::CheckInRequired);
        assert_eq!(check(&v.token("validTrial"), &fp, iat, exp, &key).status, Status::Expired);
    }

    /// A licence for another of the vendor's products must not carry this
    /// one. The vectors are all `vizz`, so asking for anything else is the
    /// mismatch.
    #[test]
    fn another_products_token_is_invalid_here() {
        let v = Vectors::load();
        let (key, fp) = (v.key(), v.fingerprint());
        let now = v.int(&["now"]);
        let iat = v.int(&["claims", "iat"]);
        let ours = check_for(&v.token("valid"), &fp, iat, now, &key, "vizz");
        assert_eq!(ours.status, Status::Active);
        for other in ["light", "datamosh", "crewbox", "yewee", "Vizz", ""] {
            let theirs = check_for(&v.token("valid"), &fp, iat, now, &key, other);
            assert_eq!(theirs.status, Status::Invalid, "a vizz token licensed {other:?}");
            assert!(theirs.claims.is_none(), "an unusable token's claims were handed on");
        }
        // Whatever else is wrong with it — a lapsed trial, another
        // machine — another product's token is simply not ours.
        let trial = check_for(&v.token("validTrial"), &fp, iat, 1_762_678_400, &key, "light");
        assert_eq!(trial.status, Status::Invalid);
        let moved = check_for(&v.token("valid"), "ELSEWHERE", iat, now, &key, "light");
        assert_eq!(moved.status, Status::Invalid);
    }

    #[test]
    fn a_wrong_machine_keeps_its_claims_for_the_panel() {
        let v = Vectors::load();
        let verdict = check(&v.token("valid"), "SOME-OTHER-MACHINE", 0, v.int(&["now"]), &v.key());
        assert_eq!(verdict.status, Status::WrongMachine);
        assert_eq!(verdict.claims.unwrap().key, "LT-V1ZZ-K7M2-9PQR-4XTC");
    }

    #[test]
    fn the_fingerprint_is_trimmed_before_hashing_and_case_is_kept() {
        let v = Vectors::load();
        let fp = v.fingerprint();
        assert_eq!(machine_hash(&format!("  {fp}\n")), v.str(&["machineHash"]));
        // The service does not fold case, so neither may we: a lower-cased
        // IOPlatformUUID is another machine.
        assert_ne!(machine_hash(&fp.to_lowercase()), v.str(&["machineHash"]));
        assert_eq!(machine_hash(&fp).len(), 32);
    }

    /// The placeholder the SDK ships with, and other keys that are not a
    /// usable Ed25519 point, must never read a token as a licence.
    #[test]
    fn an_unusable_public_key_verifies_nothing() {
        let v = Vectors::load();
        for key in ["", "REPLACE_WITH_YOUR_PUBLIC_KEY_HEX", "00", &"zz".repeat(32), &"aa".repeat(32)] {
            assert!(verify(&v.token("valid"), key).is_none(), "key {key:?} accepted a token");
        }
    }

    #[test]
    fn malformed_shapes_are_rejected_rather_than_guessed() {
        let key = Vectors::load().key();
        for bad in ["", ".", "a.", ".b", "a.b.c", "not-base64!.also-not", "eyJ2IjoxfQ"] {
            assert!(verify(bad, &key).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        assert_eq!(from_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("0g"), None);
        assert_eq!(to_hex(&[0, 255, 16]), "00ff10");
    }
}
