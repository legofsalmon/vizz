//! Licence keys, as far as the app needs to understand them — which is
//! hardly at all.
//!
//! Every key the vendor issues — a shop sale, a manual key or a trial
//! from the admin panel, an in-app trial — has the one shape
//! `LT-XXXX-XXXX-XXXX-XXXX`, and the service is the authority on all of
//! them. The key goes to the service as typed (trimmed). The only thing
//! read out of it here is the first group, a product tag, for one
//! friendly sentence before the round trip: "that is a key for Light".

use crate::PRODUCT;

/// The vendor's product tags, first group of a key.
const TAGS: [(&str, &str, &str); 5] = [
    ("V1ZZ", "vizz", "Vizz"),
    ("CREW", "crewbox", "Crewbox"),
    ("DATA", "datamosh", "Datamosh"),
    ("11GH", "light", "Light"),
    ("YEWE", "yewee", "Yewee"),
];

/// A key in its canonical form, if the input has a key's shape.
///
/// The service's own folding: case is ignored, `I` and `L` read as `1`,
/// `O` as `0`, `U` as `V`, and the `LT-` prefix is optional. Spaces are
/// dropped, since keys get pasted out of emails. The checksum group is
/// not verified — that is the service's job, and it says so by name
/// (`malformed_key`).
pub fn normalise(input: &str) -> Option<String> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let upper = compact.to_ascii_uppercase();
    let body = upper.strip_prefix("LT-").unwrap_or(&upper);
    let groups: Vec<String> = if body.contains('-') {
        body.split('-').map(fold).collect()
    } else {
        // Pasted without dashes: sixteen characters, or eighteen with LT.
        let bare = if body.len() == 18 { body.strip_prefix("LT").unwrap_or(body) } else { body };
        if bare.len() != 16 || !bare.is_ascii() {
            return None;
        }
        (0..4).map(|i| fold(&bare[i * 4..i * 4 + 4])).collect()
    };
    let shaped = groups.len() == 4
        && groups.iter().all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_alphanumeric()));
    shaped.then(|| format!("LT-{}", groups.join("-")))
}

fn fold(group: &str) -> String {
    group
        .chars()
        .map(|c| match c {
            'I' | 'L' => '1',
            'O' => '0',
            'U' => 'V',
            other => other,
        })
        .collect()
}

/// A sentence for a key that is plainly for another of the vendor's
/// products, or `None` when it is not (or cannot be told).
///
/// Advice, not a gate: the key is still sent if the person presses
/// Activate, and the service's `wrong_product` answer is the authority.
pub fn other_product_hint(input: &str) -> Option<String> {
    let key = normalise(input)?;
    let tag = &key[3..7];
    let (_, product, name) = TAGS.iter().find(|(t, _, _)| *t == tag)?;
    (*product != PRODUCT).then(|| format!("that looks like a key for {name}, not Vizz"))
}

/// A key as the panel shows it: enough to recognise, not enough to copy
/// off a screen share. `LT-V1ZZ-····-····-4XTC`.
pub fn masked(key: &str) -> String {
    match normalise(key) {
        Some(k) => format!("{}-····-····-{}", &k[..7], &k[k.len() - 4..]),
        None => "a key".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_the_way_the_service_does() {
        let want = Some("LT-V1ZZ-K7M2-9PQR-4XTC".to_string());
        assert_eq!(normalise("LT-V1ZZ-K7M2-9PQR-4XTC"), want);
        assert_eq!(normalise("lt-v1zz-k7m2-9pqr-4xtc"), want, "lower case");
        assert_eq!(normalise("V1ZZ-K7M2-9PQR-4XTC"), want, "no LT-");
        assert_eq!(normalise("  LT-VIZZ-K7M2-9PQR-4XTC \n"), want, "I folds to 1");
        assert_eq!(normalise("LT-ULZZ-K7M2-9PQR-4XTC"), want, "U to V, L to 1");
        assert_eq!(normalise("LTV1ZZK7M29PQR4XTC"), want, "no dashes at all");
        assert_eq!(normalise("V1ZZ K7M2 9PQR 4XTC"), want, "spaced out, as read aloud");
        assert_eq!(normalise("LT-V1ZZ-K7M2-9PQR"), None, "a group short");
        assert_eq!(normalise("LT-V1ZZ-K7M2-9PQR-4XTC-AAAA"), None, "a group over");
        assert_eq!(normalise("LT-V1ZZ-K7M2-9PQR-4XT!"), None);
        assert_eq!(normalise(""), None);
        assert_eq!(normalise("LT-O0OO-0000-0000-0000").as_deref(), Some("LT-0000-0000-0000-0000"));
    }

    #[test]
    fn names_the_product_a_foreign_key_belongs_to() {
        assert_eq!(other_product_hint("LT-V1ZZ-K7M2-9PQR-4XTC"), None, "ours");
        assert_eq!(other_product_hint("lt-vizz-k7m2-9pqr-4xtc"), None, "ours, typed loosely");
        assert!(other_product_hint("LT-11GH-AAAA-BBBB-CCCC").unwrap().contains("Light"));
        // Typed as a word, folded into the tag.
        assert!(other_product_hint("LT-LIGH-AAAA-BBBB-CCCC").unwrap().contains("Light"));
        assert!(other_product_hint("LT-DATA-AAAA-BBBB-CCCC").unwrap().contains("Datamosh"));
        assert!(other_product_hint("LT-CREW-AAAA-BBBB-CCCC").unwrap().contains("Crewbox"));
        assert!(other_product_hint("LT-YEWE-AAAA-BBBB-CCCC").unwrap().contains("Yewee"));
        // An unknown tag or a half-typed key says nothing rather than
        // guessing.
        assert_eq!(other_product_hint("LT-ZZZZ-AAAA-BBBB-CCCC"), None);
        assert_eq!(other_product_hint("LT-DATA"), None);
    }

    #[test]
    fn a_masked_key_keeps_the_ends() {
        assert_eq!(masked("lt-v1zz-k7m2-9pqr-4xtc"), "LT-V1ZZ-····-····-4XTC");
        assert_eq!(masked("junk"), "a key");
    }
}
