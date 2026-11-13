#!/usr/bin/env python3
"""Live acceptance for `libpagh::process` (Command-lite over fork/execve/wait4).

The harness builds `examples/spawn.rs` with the pinned toolchain, boots the
pagh-riscv kernel from a checkout's own ``tools/run.py`` (the kernel repository
is never modified) with an ext2 image built by ``mke2fs -d`` containing the
example as ``/spawn`` (the guest sees ``/mnt/spawn``), and checks the serial
transcript line by line. The transcript is copied to ``docs/live-spawn.txt``.

The example re-executes itself as the child, so the run needs no host peers and
no shell fixtures: it proves `Command::status`/`spawn`, `Child::id`/`try_wait`/
`wait`, exit-code and signal decoding, the exec-failure path (fd 2 message and
exit 127) and `env`/`env_clear`.

Usage::

    python3 tools/live-spawn.py

``PAGH_RISCV`` selects the kernel checkout (default ``../pagh-riscv``); its
release ELF must already be built. QEMU, mke2fs and the pinned Rust toolchain
(``rust-toolchain.toml``) are required.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
KERNEL_RELEASE = "target/riscv64gc-unknown-none-elf/release/pagh"
KERNEL_DEBUG = "target/riscv64gc-unknown-none-elf/debug/pagh"
EXAMPLE = "examples/spawn"
TARGET = "riscv64gc-unknown-none-elf"
TIMEOUT = 180.0

# Every marker the transcript must contain. `try_wait` is a race by nature: the
# child may have exited before the parent asks, so that line has alternatives.
EXPECTED = (
    "spawn: child pid ",
    "spawn: status() -> exit code 42 (raw 10752)",
    "spawn: spawn() -> child pid ",
    "spawn: wait() -> exit code 42 equal to try_wait/first status",
    "/mnt/spawn-no-such-program: No such file or directory",
    "spawn: missing program -> exit code 127 (execve error printed above)",
    "spawn: child sees PAGH_SPAWN_TEST=42",
    "spawn: env override reached the child",
    "spawn: child sees PAGH_SPAWN_TEST=7",
    "spawn: env_clear + env reached the child",
    "spawn: ok (status/spawn/try_wait/wait/env/env_clear)",
    "run: /mnt/spawn exited with code 0",
)
ANY_OF = (
    (
        "spawn: try_wait() -> still running",
        "spawn: try_wait() reaped immediately",
    ),
)


def die(message: str, code: int = 2) -> None:
    print(f"live-spawn: {message}", file=sys.stderr)
    raise SystemExit(code)


def git_revision(repo: pathlib.Path) -> str:
    try:
        head = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--short", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        dirty = subprocess.run(
            ["git", "-C", str(repo), "status", "--porcelain"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        return head + (" (dirty)" if dirty else " (clean)")
    except (OSError, subprocess.CalledProcessError) as error:
        return f"unknown ({error})"


def crate_version(repo: pathlib.Path) -> str:
    try:
        text = (repo / "Cargo.toml").read_text(encoding="utf-8")
    except OSError:
        return "unknown"
    for line in text.splitlines():
        if line.startswith("version"):
            return line.split("=", 1)[1].strip().strip('"')
    return "unknown"


def rustc_version() -> str:
    try:
        return subprocess.run(
            ["rustc", "-V"], check=True, capture_output=True, text=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def qemu_version() -> str:
    try:
        return subprocess.run(
            ["qemu-system-riscv64", "--version"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()[0]
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--transcript",
        default=str(ROOT / "docs" / "live-spawn.txt"),
        help="where to write the committed transcript",
    )
    parser.add_argument(
        "--pagh-riscv",
        default=os.environ.get("PAGH_RISCV", str(ROOT.parent / "pagh-riscv")),
        help="pagh-riscv checkout (default ../pagh-riscv or $PAGH_RISCV)",
    )
    parser.add_argument("--keep-workdir", action="store_true")
    args = parser.parse_args()

    kernel_repo = pathlib.Path(args.pagh_riscv).resolve()
    kernel = kernel_repo / KERNEL_RELEASE
    if not kernel.is_file():
        kernel = kernel_repo / KERNEL_DEBUG
    if not kernel.is_file():
        die(f"no built kernel under {kernel_repo} (run `cargo build --release` there)")
    run_py = kernel_repo / "tools" / "run.py"
    if not run_py.is_file():
        die(f"{kernel_repo} is not a pagh-riscv checkout (tools/run.py missing)")
    for tool in ("qemu-system-riscv64", "mke2fs", "cargo"):
        if shutil.which(tool) is None:
            die(f"{tool} is required")

    work = pathlib.Path(tempfile.mkdtemp(prefix="pagh-live-spawn-"))
    try:
        print("live-spawn: building examples/spawn.rs", file=sys.stderr)
        build = subprocess.run(
            ["cargo", "build", "--release", "--example", "spawn", "--target", TARGET],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if build.returncode != 0:
            print(build.stdout, file=sys.stderr)
            print(build.stderr, file=sys.stderr)
            die("cargo build failed")

        example = ROOT / "target" / TARGET / "release" / "examples" / "spawn"
        if not example.is_file():
            die(f"{example} was not built")

        diskroot = work / "root"
        diskroot.mkdir()
        shutil.copy(example, diskroot / "spawn")

        disk = work / "disk.img"
        make = subprocess.run(
            [
                "mke2fs",
                "-q",
                "-t",
                "ext2",
                "-b",
                "1024",
                "-F",
                "-d",
                str(diskroot),
                str(disk),
                "16M",
            ],
            capture_output=True,
            text=True,
        )
        if make.returncode != 0:
            die(f"mke2fs failed: {make.stderr.strip()}")

        serial = work / "serial.sock"
        monitor = work / "monitor.sock"
        shot = work / "shot.png"
        log = work / "serial.log"
        command = [
            sys.executable,
            str(run_py),
            "--kernel",
            str(kernel),
            "--disk",
            str(disk),
            "--keys",
            "run /mnt/spawn\\n",
            "--settle",
            "8",
            "--serial",
            str(serial),
            "--monitor",
            str(monitor),
            "--serial-log",
            str(log),
            "--shot",
            str(shot),
            "--timeout",
            str(TIMEOUT),
        ]
        print("live-spawn: booting the kernel", file=sys.stderr)
        run = subprocess.run(command, capture_output=True, text=True)
        if run.returncode != 0:
            print(run.stdout, file=sys.stderr)
            print(run.stderr, file=sys.stderr)

        transcript = log.read_text(encoding="utf-8", errors="replace") if log.is_file() else ""
        header = "\n".join(
            [
                "# Live Command-lite acceptance for libpagh (process::Command over fork/execve/wait4)",
                f"# kernel: {kernel}",
                f"# kernel checkout: {kernel_repo} (revision {git_revision(kernel_repo)}, crate {crate_version(kernel_repo)})",
                f"# kernel sha256: {sha256(kernel)}",
                f"# pagh-api revision: {git_revision(ROOT)}",
                f"# host: {qemu_version()}, {rustc_version()}",
                "# image: mke2fs -t ext2 -b 1024 -d <root with target/.../examples/spawn as spawn> disk.img",
                "# guest command: run /mnt/spawn",
                "",
            ]
        )
        output = header + transcript
        if not transcript.endswith("\n"):
            output += "\n"
        pathlib.Path(args.transcript).write_text(output, encoding="utf-8")

        missing = [marker for marker in EXPECTED if marker not in transcript]
        missing_groups = [
            group for group in ANY_OF if not any(alt in transcript for alt in group)
        ]
        if run.returncode != 0 or missing or missing_groups:
            for marker in missing:
                print(f"live-spawn: MISSING: {marker}", file=sys.stderr)
            for group in missing_groups:
                print(f"live-spawn: MISSING one of: {group}", file=sys.stderr)
            print(f"live-spawn: transcript is {args.transcript}", file=sys.stderr)
            return 1
        print(f"live-spawn: PASS (transcript {args.transcript})", file=sys.stderr)
        return 0
    finally:
        if args.keep_workdir:
            print(f"live-spawn: workdir kept at {work}", file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())