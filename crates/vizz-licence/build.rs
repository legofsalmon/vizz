//! Stamps the build's own date into the binary.
//!
//! A licence's `maintUntil` means "a build released at or before this
//! keeps working forever", so the date it is compared against has to be
//! the moment this binary was made, fixed inside it. Never a file mtime:
//! copying or re-downloading the app silently rewrites that, and anyone
//! can edit it.
//!
//! `VIZZ_BUILD_DATE` (unix seconds) pins it, for a release job that wants
//! the tag's date rather than the runner's clock; `SOURCE_DATE_EPOCH` is
//! honoured for the same reason reproducible-build tooling sets it.
//! Otherwise it is now.
//!
//! Cargo only re-runs this when one of those variables changes, so an
//! incremental debug build can carry an older date than the moment it was
//! linked. That is the harmless direction — an older build is entitled to
//! more, never less — and the release job builds from clean.

fn main() {
    println!("cargo:rerun-if-env-changed=VIZZ_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // Rebuild when the verifying key changes, so switching keys cannot
    // leave a stale constant in an otherwise fresh binary.
    println!("cargo:rerun-if-env-changed=VIZZ_LICENCE_PUBLIC_KEY");

    let pinned = ["VIZZ_BUILD_DATE", "SOURCE_DATE_EPOCH"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find_map(|v| v.trim().parse::<u64>().ok());
    let stamp = pinned.unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    });
    println!("cargo:rustc-env=VIZZ_LICENCE_BUILD_DATE={stamp}");
}
