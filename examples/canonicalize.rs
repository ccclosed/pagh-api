//! Self-checking tour of the 0.9.0 path/fs helpers: `fs::canonicalize`
//! (component-wise `realpath` over `readlinkat` + `stat`), the std-shaped
//! `path` component helpers and the `io::stdout()` handle constructor.
//!
//! The kernel takes absolute paths only and has no `realpath` syscall, so this
//! builds a small tree on the writable ext2 mount and checks both the value
//! results and the documented error cases (missing target, broken link,
//! symlink loop, file used as a directory prefix). The interesting case is the
//! `deep/../file.txt` one: `deep` is a symlink to `sub/x`, so `..` must be
//! applied *after* following it (`sub/file.txt`), not lexically before
//! (`file.txt`) — which is exactly what `path::normalize` alone cannot do.
//!
//! ```sh
//! cargo build --release --example canonicalize --target riscv64gc-unknown-none-elf
//! run /mnt/canonicalize
//! ```
//!
//! Exit codes: 0 success, 1 setup, 2 canonicalize values, 3 error cases,
//! 4 path helpers, 5 io handle.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use libpagh::errno::Errno;
use libpagh::fs;
use libpagh::io::{self, Write};
use libpagh::path::Path;
use libpagh::println;

const ROOT: &str = "/mnt/canon-test";
const SUB: &str = "/mnt/canon-test/sub";
const SUB_FILE: &str = "/mnt/canon-test/sub/file.txt";

fn setup() -> bool {
    let mut ok = true;
    let create = |path: &str| match fs::create_dir_all(path) {
        Ok(()) => true,
        Err(error) => {
            println!("canonicalize: create_dir_all({path}) failed: {error}");
            false
        }
    };
    ok &= create(ROOT);
    ok &= create(SUB);
    ok &= create("/mnt/canon-test/sub/x");
    let write = |path: &str, body: &str| match fs::write(path, body.as_bytes()) {
        Ok(()) => true,
        Err(error) => {
            println!("canonicalize: write({path}) failed: {error}");
            false
        }
    };
    ok &= write("/mnt/canon-test/file.txt", "root\n");
    ok &= write(SUB_FILE, "sub\n");
    let link = |target: &str, path: &str| match fs::symlink(target, path) {
        Ok(()) => true,
        Err(Errno::EEXIST) => true, // a rerun on the same image
        Err(error) => {
            println!("canonicalize: symlink({target}, {path}) failed: {error}");
            false
        }
    };
    ok &= link("sub", "/mnt/canon-test/linkdir");
    ok &= link("/mnt/canon-test/sub", "/mnt/canon-test/absdir");
    ok &= link("sub/x", "/mnt/canon-test/deep");
    ok &= link("loop", "/mnt/canon-test/loop");
    ok &= link("missing-target", "/mnt/canon-test/broken");
    ok
}

fn expect(label: &str, got: Result<String, Errno>, want: &str) -> bool {
    match got {
        Ok(value) if value == want => {
            println!("canonicalize: {label} -> {value}");
            true
        }
        Ok(value) => {
            println!("canonicalize: {label} -> {value}, expected {want}");
            false
        }
        Err(error) => {
            println!("canonicalize: {label} failed: {error}, expected {want}");
            false
        }
    }
}

fn expect_error(label: &str, got: Result<String, Errno>, want: Errno) -> bool {
    match got {
        Err(error) if error == want => {
            println!("canonicalize: {label} -> {error} (expected)");
            true
        }
        Err(error) => {
            println!("canonicalize: {label} -> {error}, expected {want}");
            false
        }
        Ok(value) => {
            println!("canonicalize: {label} unexpectedly resolved to {value}");
            false
        }
    }
}

fn check_values() -> bool {
    let mut ok = true;
    ok &= expect("root", fs::canonicalize("/"), "/");
    ok &= expect(
        "dot-dot-lexical",
        fs::canonicalize(&format!("{ROOT}/sub/../sub/file.txt")),
        SUB_FILE,
    );
    ok &= expect(
        "dots-and-slashes",
        fs::canonicalize(&format!("{ROOT}/./sub//file.txt")),
        SUB_FILE,
    );
    ok &= expect(
        "relative-link",
        fs::canonicalize(&format!("{ROOT}/linkdir/file.txt")),
        SUB_FILE,
    );
    ok &= expect(
        "absolute-link",
        fs::canonicalize(&format!("{ROOT}/absdir/file.txt")),
        SUB_FILE,
    );
    ok &= expect(
        "symlink-then-dot-dot",
        fs::canonicalize(&format!("{ROOT}/deep/../file.txt")),
        SUB_FILE,
    );
    ok &= expect(
        "link-dot-dot-link",
        fs::canonicalize(&format!("{ROOT}/linkdir/../linkdir/file.txt")),
        SUB_FILE,
    );
    ok &= expect("directory", fs::canonicalize(ROOT), ROOT);
    ok
}

fn check_errors() -> bool {
    let mut ok = true;
    ok &= expect_error(
        "missing-component",
        fs::canonicalize(&format!("{ROOT}/nope")),
        Errno::ENOENT,
    );
    ok &= expect_error(
        "broken-link",
        fs::canonicalize(&format!("{ROOT}/broken")),
        Errno::ENOENT,
    );
    ok &= expect_error(
        "symlink-loop",
        fs::canonicalize(&format!("{ROOT}/loop")),
        Errno::ELOOP,
    );
    ok &= expect_error(
        "file-as-directory",
        fs::canonicalize(&format!("{SUB_FILE}/x")),
        Errno::ENOTDIR,
    );
    ok
}

fn check_path_helpers() -> bool {
    let mut ok = true;
    let mut check = |what: &str, condition: bool| {
        if condition {
            println!("canonicalize: Path::{what} ok");
        } else {
            println!("canonicalize: Path::{what} FAILED");
            ok = false;
        }
    };
    let path = Path::new("/a/b/c.tar.gz");
    check("file_name", path.file_name() == Some("c.tar.gz"));
    check("extension", path.extension() == Some("gz"));
    check("parent", path.parent() == Some("/a/b"));
    check(
        "starts_with",
        path.starts_with("/a") && !path.starts_with("/a/bc"),
    );
    check("root-file-name", Path::new("/").file_name().is_none());
    check(
        "hidden-extension",
        Path::new(".hidden").extension().is_none(),
    );
    check("trailing-dot", Path::new("a.").extension() == Some(""));
    check("root-parent", Path::new("/").parent().is_none());
    check("relative-parent", Path::new("a").parent() == Some(""));
    check(
        "canonicalize-method",
        matches!(
            Path::new(&format!("{ROOT}/linkdir/file.txt")).canonicalize(),
            Ok(value) if value.as_str() == SUB_FILE
        ),
    );
    ok
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let mut out = io::stdout();
    if out
        .write_all(b"canonicalize: io::stdout() handle writes\n")
        .is_err()
    {
        return 5;
    }
    if !setup() {
        return 1;
    }
    println!("canonicalize: tree ready under {ROOT}");
    if !check_values() {
        return 2;
    }
    if !check_errors() {
        return 3;
    }
    if !check_path_helpers() {
        return 4;
    }
    println!("canonicalize: ok (realpath values, errors, Path helpers, io::stdout)");
    0
}
