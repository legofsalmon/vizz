//! What the licence section says, in words, decided here so it can be
//! tested without drawing anything.

use crate::verify::{Status, Verdict};

const DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Licensed, or a trial running.
    Good,
    /// Worth knowing, never a problem: a lease to renew, an update window
    /// that closed, a build that checks nothing.
    Note,
    /// Not licensed.
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headline {
    pub text: String,
    pub tone: Tone,
}

/// The one line: "Licensed to …", "Trial, 12 days left", "Trial ended",
/// "Unlicensed", "Updates ended 21 Aug 2027", "Needs check-in".
pub fn headline(verdict: &Verdict, now: i64, key_configured: bool) -> Headline {
    let line = |text: String, tone| Headline { text, tone };
    if !key_configured {
        return line("Licence checks are off in this build".into(), Tone::Note);
    }
    let claims = verdict.claims.as_ref();
    match verdict.status {
        Status::Active => match claims {
            Some(c) if c.is_trial() => line(format!("Trial, {}", days_left(c.exp - now)), Tone::Good),
            Some(c) => match c.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(name) => line(format!("Licensed to {name}"), Tone::Good),
                None => line("Licensed".into(), Tone::Good),
            },
            None => line("Licensed".into(), Tone::Good),
        },
        Status::UpdateRequired => match claims {
            Some(c) => line(format!("Updates ended {}", date(c.maint_until)), Tone::Note),
            None => line("Updates ended".into(), Tone::Note),
        },
        Status::CheckInRequired => line("Needs check-in".into(), Tone::Note),
        Status::Expired => line("Trial ended".into(), Tone::Bad),
        Status::WrongMachine => line("Licensed to another machine".into(), Tone::Bad),
        Status::Invalid => line("Unlicensed".into(), Tone::Bad),
    }
}

/// The lines under the headline: what it means and what to do.
pub fn details(verdict: &Verdict, key_configured: bool, has_token: bool) -> Vec<String> {
    if !key_configured {
        return vec!["this build has no key to verify a licence with, so nothing is restricted".into()];
    }
    let mut out = Vec::new();
    let Some(c) = verdict.claims.as_ref() else {
        if has_token {
            out.push("the stored licence did not verify on this build".into());
        }
        return out;
    };
    match verdict.status {
        Status::Active if c.is_trial() => {
            out.push(format!("the trial ends {}", date(c.exp)));
        }
        Status::Active | Status::UpdateRequired => {
            out.push(format!("updates until {}", date(c.maint_until)));
            out.push(format!("checks in by {}", date(c.exp)));
            if verdict.status == Status::UpdateRequired {
                out.push("this build came out after the update window — it keeps working, and a renewal brings newer ones".into());
            }
        }
        Status::CheckInRequired => {
            out.push(format!(
                "not checked in since {} — nothing is restricted; vizz checks in by itself once it is online",
                date(c.exp)
            ));
            out.push(format!("updates until {}", date(c.maint_until)));
        }
        Status::Expired => out.push(format!("the trial ended {}", date(c.exp))),
        Status::WrongMachine => out.push(format!(
            "this licence was activated on machine {} — release it from your account, then activate here",
            short(&c.machine)
        )),
        Status::Invalid => {}
    }
    if c.seats > 1 && !c.is_trial() {
        out.push(format!("{} seats", c.seats));
    }
    out
}

/// "12 days left", "1 day left", "ends today".
pub fn days_left(secs: i64) -> String {
    if secs <= 0 {
        return "ended".into();
    }
    if secs < DAY {
        return "ends today".into();
    }
    let days = (secs + DAY - 1) / DAY;
    if days == 1 { "1 day left".into() } else { format!("{days} days left") }
}

/// The first eight characters of a machine hash, for showing.
pub fn short(machine: &str) -> &str {
    machine.get(..8).unwrap_or(machine)
}

/// A unix time as "21 Aug 2027", in UTC. Hand-rolled because it is the
/// only date this app formats, and a calendar crate for one line would be
/// the largest thing in this one.
pub fn date(unix: i64) -> String {
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d) = civil_from_days(unix.div_euclid(DAY));
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::Claims;

    fn claims(edition: &str, name: Option<&str>) -> Claims {
        Claims {
            v: 1,
            key: "LT-V1ZZ-K7M2-9PQR-4XTC".into(),
            product: "vizz".into(),
            edition: edition.into(),
            customer: "c".into(),
            name: name.map(str::to_string),
            seats: 2,
            maint_until: 1_787_270_400, // 21 Aug 2026
            exp: 1_000_000 + 12 * DAY,
            machine: "8b9dd6da2bcf47bdfe7ceb27c2a58680".into(),
            mode: "online".into(),
            iat: 0,
            jti: "j".into(),
        }
    }

    fn verdict(status: Status, c: Option<Claims>) -> Verdict {
        Verdict { status, claims: c, check_in_in: None }
    }

    #[test]
    fn dates_read_the_way_people_write_them() {
        assert_eq!(date(0), "1 Jan 1970");
        assert_eq!(date(1_787_270_400), "21 Aug 2026");
        assert_eq!(date(1_709_164_800), "29 Feb 2024", "a leap day");
        assert_eq!(date(1_791_536_000), "9 Oct 2026");
        assert_eq!(date(-DAY), "31 Dec 1969");
    }

    #[test]
    fn every_status_has_the_headline_the_brief_names() {
        let now = 1_000_000;
        let h = |s, c| headline(&verdict(s, c), now, true);
        assert_eq!(h(Status::Active, Some(claims("standard", Some("Colly")))).text, "Licensed to Colly");
        assert_eq!(h(Status::Active, Some(claims("standard", None))).text, "Licensed");
        assert_eq!(h(Status::Active, Some(claims("trial", None))).text, "Trial, 12 days left");
        assert_eq!(h(Status::Expired, Some(claims("trial", None))).text, "Trial ended");
        assert_eq!(h(Status::Invalid, None).text, "Unlicensed");
        assert_eq!(
            h(Status::UpdateRequired, Some(claims("standard", None))).text,
            "Updates ended 21 Aug 2026"
        );
        assert_eq!(h(Status::CheckInRequired, Some(claims("standard", None))).text, "Needs check-in");
        assert_eq!(
            h(Status::WrongMachine, Some(claims("standard", None))).text,
            "Licensed to another machine"
        );
        // The notes are notes, not alarms.
        assert_eq!(h(Status::UpdateRequired, Some(claims("standard", None))).tone, Tone::Note);
        assert_eq!(h(Status::CheckInRequired, Some(claims("standard", None))).tone, Tone::Note);
        // And a build that cannot check says so rather than "Unlicensed".
        assert_eq!(headline(&verdict(Status::Invalid, None), now, false).tone, Tone::Note);
    }

    #[test]
    fn days_left_rounds_up_and_ends_today() {
        assert_eq!(days_left(30 * DAY), "30 days left");
        assert_eq!(days_left(29 * DAY + 1), "30 days left");
        assert_eq!(days_left(DAY + 1), "2 days left");
        assert_eq!(days_left(DAY), "1 day left");
        assert_eq!(days_left(3600), "ends today");
        assert_eq!(days_left(0), "ended");
    }

    #[test]
    fn a_wrong_machine_names_the_machine_it_is_for() {
        let d = details(&verdict(Status::WrongMachine, Some(claims("standard", None))), true, true);
        assert!(d[0].contains("8b9dd6da"), "{d:?}");
    }
}
