//! Cargo does not track a linker script passed through `-C link-arg=-T…`, so a
//! change to `pagh-user.ld` would silently leave a stale ELF. Re-run (and thus
//! relink dependents) when the script changes.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../pagh-user.ld");
}