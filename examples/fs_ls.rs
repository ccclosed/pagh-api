//! End-to-end tour of the `libpagh::fs` helpers added in 0.7.0:
//! [`fs::write`], [`fs::read_to_string`] and [`fs::read_dir`].
//!
//! It is its own acceptance check: it writes a file into the directory, reads
//! it back, lists the directory, checks that the entry appears with the right
//! kind, path and contents (`DirEntry::path()` → `read_to_string`), removes
//! the file again and only then prints `fs_ls: ok`. A missing or wrong step
//! returns a non-zero exit code, so the QEMU transcript can assert on both the
//! markers and `run: /mnt/fs_ls exited with code 0`.
//!
//! ```sh
//! cargo build --release --example fs_ls --target riscv64gc-unknown-none-elf
//! # in the guest shell (the ext2 image is mounted at /mnt):
//! run /mnt/fs_ls /mnt
//! ```
//!
//! With no argument it lists the current directory (`PWD`).
//!
//! Exit codes: 0 success, 1 write, 2 read_to_string, 3 directory listing,
//! 4 probe entry missing/incorrect, 5 remove.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libpagh::fs;
use libpagh::println;

const PROBE: &str = ".fs_ls-probe";
const BODY: &str = "pagh fs helpers\nwrite + read_to_string + read_dir\n";

fn join(dir: &str, name: &str) -> String {
    let mut path = String::with_capacity(dir.len() + 1 + name.len());
    path.push_str(dir);
    if !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(name);
    path
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let dir = libpagh::env::args()
        .nth(1)
        .unwrap_or_else(libpagh::env::current_dir);
    let probe = join(&dir, PROBE);

    // 1. fs::write creates the file and writes every byte.
    if let Err(error) = fs::write(&probe, BODY.as_bytes()) {
        println!("fs_ls: write {probe} failed: {error}");
        return 1;
    }
    println!("fs_ls: wrote {probe} ({} bytes)", BODY.len());

    // 2. fs::read_to_string reads it back unchanged.
    match fs::read_to_string(&probe) {
        Ok(text) if text == BODY => println!("fs_ls: read_to_string ok"),
        Ok(text) => {
            println!("fs_ls: read_to_string mismatch ({} bytes)", text.len());
            return 2;
        }
        Err(error) => {
            println!("fs_ls: read_to_string failed: {error}");
            return 2;
        }
    }

    // 3. fs::read_dir yields DirEntry values; `path()` is the full path, so it
    //    feeds straight back into the other helpers.
    let mut entries: Vec<fs::DirEntry> = Vec::new();
    match fs::read_dir(&dir) {
        Ok(iter) => {
            for entry in iter {
                match entry {
                    Ok(entry) => entries.push(entry),
                    Err(error) => {
                        println!("fs_ls: read_dir({dir}) failed: {error}");
                        return 3;
                    }
                }
            }
        }
        Err(error) => {
            println!("fs_ls: read_dir({dir}) failed: {error}");
            return 3;
        }
    }
    for entry in &entries {
        let suffix = if entry.file_type().is_dir() {
            "/"
        } else if entry.file_type().is_symlink() {
            "@"
        } else {
            ""
        };
        println!("fs_ls: {}{suffix}", entry.file_name());
    }

    // 4. The probe must be one of the entries, with a regular-file kind and the
    //    full path that reads back the same bytes.
    match entries.iter().find(|entry| entry.file_name() == PROBE) {
        None => {
            println!("fs_ls: {PROBE} missing from listing");
            return 4;
        }
        Some(entry) => {
            println!(
                "fs_ls: {PROBE}: ino {} kind {:?} path {}",
                entry.ino(),
                entry.file_type(),
                entry.path()
            );
            if !entry.file_type().is_file() {
                println!("fs_ls: {PROBE} is not a regular file");
                return 4;
            }
            if entry.path() != probe {
                println!(
                    "fs_ls: DirEntry::path is {}, expected {probe}",
                    entry.path()
                );
                return 4;
            }
            match fs::read_to_string(entry.path()) {
                Ok(text) if text == BODY => println!("fs_ls: DirEntry::path round-trip ok"),
                Ok(_) => {
                    println!("fs_ls: DirEntry::path content mismatch");
                    return 4;
                }
                Err(error) => {
                    println!("fs_ls: DirEntry::path read failed: {error}");
                    return 4;
                }
            }
            match entry.metadata() {
                Ok(stat) if stat.is_file() && stat.size == BODY.len() as u64 => {
                    println!("fs_ls: metadata ok (size {})", stat.size)
                }
                Ok(stat) => {
                    println!(
                        "fs_ls: metadata unexpected: kind {:?} size {}",
                        stat.entry_kind(),
                        stat.size
                    );
                    return 4;
                }
                Err(error) => {
                    println!("fs_ls: metadata failed: {error}");
                    return 4;
                }
            }
        }
    }

    // 5. fs::remove_file leaves the directory as it was.
    match fs::remove_file(&probe) {
        Ok(()) => println!("fs_ls: removed {probe}"),
        Err(error) => {
            println!("fs_ls: remove_file failed: {error}");
            return 5;
        }
    }

    println!(
        "fs_ls: ok ({} entries in {dir}, read_dir + read_to_string + write)",
        entries.len()
    );
    0
}
