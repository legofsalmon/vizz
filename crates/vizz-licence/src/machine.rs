//! This machine's identity, as the licence service knows it.
//!
//! The RAW platform id is what goes on the wire — the service hashes it
//! (`sha256(trimmed)[:32]` hex) and signs that hash into the token. The
//! app hashes it nowhere except to compare against a token's claim and to
//! show a short id. Sending the hash instead mints tokens for
//! `sha256(sha256(id))`, a machine that does not exist, and every check
//! from then on says `wrong_machine` — permanently, and releasing the seat
//! does not help. That exact bug cost Light a release.
//!
//! Deliberately not a MAC address: those change with docks, VPNs and USB
//! adapters, and a fingerprint that moves burns a seat every time someone
//! plugs into a different desk.
//!
//! Also the offline "request code": the account page takes exactly this
//! string and returns a token for it.

/// The raw platform id, trimmed. `None` when the platform will not say,
/// which the licence section reports rather than activating under the
/// hash of an empty string — every other machine in the same state would
/// claim that seat too.
///
/// Spawns `ioreg` on macOS and `reg` on Windows, so it is called once,
/// at startup, and cached; never per frame.
pub fn fingerprint() -> Option<String> {
    platform_id().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(target_os = "macos")]
fn platform_id() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    parse_ioreg(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(windows)]
fn platform_id() -> Option<String> {
    use std::os::windows::process::CommandExt as _;
    // No console window flashing up behind the app.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    parse_reg_query(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_id() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .ok()
}

/// `"IOPlatformUUID" = "9E5B4C1A-…"` → the UUID, case untouched: the
/// service does not fold case, so neither may this.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ioreg(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains("\"IOPlatformUUID\""))
        .and_then(|l| l.split('"').nth(3))
        .map(str::to_string)
}

/// `    MachineGuid    REG_SZ    8f2c…` → the GUID.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_query(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.trim_start().starts_with("MachineGuid"))
        .and_then(|l| l.split_whitespace().nth(2))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_uuid_out_of_ioreg() {
        let text = r#"+-o J316sAP  <class IOPlatformExpertDevice, id 0x100000240, registered>
    {
      "IOPlatformSerialNumber" = "C02XXXXXX"
      "IOPlatformUUID" = "9E5B4C1A-0000-4000-8000-ABCDEF012345"
      "manufacturer" = <"Apple Inc.">
    }"#;
        assert_eq!(parse_ioreg(text).as_deref(), Some("9E5B4C1A-0000-4000-8000-ABCDEF012345"));
        assert_eq!(parse_ioreg("nothing here"), None);
    }

    #[test]
    fn reads_the_guid_out_of_reg_query() {
        let text = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    8f2c1d3e-1111-2222-3333-444455556666\r\n\r\n";
        assert_eq!(parse_reg_query(text).as_deref(), Some("8f2c1d3e-1111-2222-3333-444455556666"));
        assert_eq!(parse_reg_query("ERROR: The system was unable to find the specified registry key"), None);
    }

    /// Stable across calls, or every launch would look like a new machine.
    /// Not asserted present: a CI container may have no machine id at all,
    /// which is exactly the case `None` exists for.
    #[test]
    fn the_fingerprint_is_stable() {
        assert_eq!(fingerprint(), fingerprint());
        if let Some(fp) = fingerprint() {
            assert_eq!(fp, fp.trim(), "not trimmed");
            assert!(!fp.is_empty());
        }
    }
}
