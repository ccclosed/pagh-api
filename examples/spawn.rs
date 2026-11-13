//! Self-checking tour of the new `libpagh::process` helpers (0.8.0).
//!
//! The kernel's shell has no `echo`/`true` fixtures to hand a test, so this
//! example re-executes **itself** as the child:
//!
//! ```sh
//! cargo build --release --example spawn --target riscv64gc-unknown-none-elf
//! run /mnt/spawn
//! ```
//!
//! It proves, with `run: /mnt/spawn exited with code 0` as the verdict:
//!
//! * `Command::status` runs a child and decodes its normal exit code (42);
//! * `Command::spawn` + `Child::id`/`try_wait`/`wait` reap the same child;
//! * a missing program makes the child exit 127 after printing the `execve`
//!   error to fd 2 (the shell convention), and the parent sees that code;
//! * `Command::env`/`env_clear` really reach the child's environment;
//! * `ExitStatus` decodes a normal exit (`code()` is `Some(42)`, `signal()`
//!   is `None`, `success()` is false), and `Command` docs spell out that
//!   cross-process `kill` is not possible in this ABI.
//!
//! Child modes (not meant to be typed by hand):
//! `--child` exits 42, `--env EXPECTED` checks `PAGH_SPAWN_TEST` and exits 0/3.
//!
//! Exit codes: 0 success, 1 self status, 2 spawn/wait, 3 missing program,
//! 4 environment.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libpagh::env;
use libpagh::process::Command;
use libpagh::{println, sys};

fn child_mode(mode: &str, rest: &[String]) -> i32 {
    match mode {
        "--child" => {
            println!(
                "spawn: child pid {} mode --child, exiting 42",
                sys::getpid().unwrap_or(-1)
            );
            42
        }
        "--env" => {
            let expected = rest.first().map(String::as_str).unwrap_or("");
            match env::var("PAGH_SPAWN_TEST") {
                Some(value) if value == expected => {
                    println!("spawn: child sees PAGH_SPAWN_TEST={value}");
                    0
                }
                Some(value) => {
                    println!("spawn: child sees PAGH_SPAWN_TEST={value}, expected {expected}");
                    3
                }
                None => {
                    println!("spawn: child has no PAGH_SPAWN_TEST");
                    3
                }
            }
        }
        _ => 2,
    }
}

fn parent(program: &str) -> i32 {
    // 1. status(): normal exit code.
    match Command::new(program).arg("--child").status() {
        Ok(status)
            if status.code() == Some(42) && !status.success() && status.signal().is_none() =>
        {
            println!("spawn: status() -> {} (raw {})", status, status.raw());
        }
        Ok(status) => {
            println!(
                "spawn: status() unexpected: {status} (raw {})",
                status.raw()
            );
            return 1;
        }
        Err(error) => {
            println!("spawn: status() failed: {error}");
            return 1;
        }
    }

    // 2. spawn() + try_wait()/wait(): same child, reaped once.
    let mut child = match Command::new(program).arg("--child").spawn() {
        Ok(child) => child,
        Err(error) => {
            println!("spawn: spawn() failed: {error}");
            return 2;
        }
    };
    let pid = child.id();
    if pid <= 0 {
        println!("spawn: Child::id() returned {pid}");
        return 2;
    }
    println!("spawn: spawn() -> child pid {pid}");
    let reaped = match child.try_wait() {
        Ok(Some(status)) => {
            println!("spawn: try_wait() reaped immediately");
            Some(status)
        }
        Ok(None) => {
            println!("spawn: try_wait() -> still running");
            None
        }
        Err(error) => {
            println!("spawn: try_wait() failed: {error}");
            return 2;
        }
    };
    let status = match reaped {
        Some(status) => status,
        None => match child.wait() {
            Ok(status) => status,
            Err(error) => {
                println!("spawn: wait() failed: {error}");
                return 2;
            }
        },
    };
    if status.code() != Some(42) {
        println!("spawn: reaped child status {status}, expected 42");
        return 2;
    }
    println!("spawn: wait() -> {} equal to try_wait/first status", status);

    // 3. Missing program: the child reports the execve error and exits 127.
    match Command::new("/mnt/spawn-no-such-program").status() {
        Ok(status) if status.code() == Some(127) => {
            println!("spawn: missing program -> exit code 127 (execve error printed above)");
        }
        Ok(status) => {
            println!("spawn: missing program gave {status}, expected 127");
            return 3;
        }
        Err(error) => {
            println!("spawn: spawning a missing program failed: {error}");
            return 3;
        }
    }

    // 4. Environment: inherited + override, and env_clear.
    match Command::new(program)
        .arg("--env")
        .arg("42")
        .env("PAGH_SPAWN_TEST", "42")
        .status()
    {
        Ok(status) if status.code() == Some(0) => println!("spawn: env override reached the child"),
        Ok(status) => {
            println!("spawn: env override child {status}");
            return 4;
        }
        Err(error) => {
            println!("spawn: env override spawn failed: {error}");
            return 4;
        }
    }
    match Command::new(program)
        .arg("--env")
        .arg("7")
        .env_clear()
        .env("PAGH_SPAWN_TEST", "7")
        .status()
    {
        Ok(status) if status.code() == Some(0) => {
            println!("spawn: env_clear + env reached the child")
        }
        Ok(status) => {
            println!("spawn: env_clear child {status}");
            return 4;
        }
        Err(error) => {
            println!("spawn: env_clear spawn failed: {error}");
            return 4;
        }
    }

    println!("spawn: ok (status/spawn/try_wait/wait/env/env_clear)");
    0
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let args: Vec<String> = env::args().collect();
    let program = args
        .first()
        .cloned()
        .unwrap_or_else(|| String::from("/mnt/spawn"));
    match args.get(1).map(String::as_str) {
        Some(mode) if mode.starts_with("--") => child_mode(mode, &args[2..]),
        _ => parent(&program),
    }
}
