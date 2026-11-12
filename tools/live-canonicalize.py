#!/usr/bin/env python3
"""Live acceptance for `libpagh` path helpers: `fs::canonicalize` over
readlinkat+stat, the std-shaped `path` component helpers and the `io` handles.

The harness builds `examples/canonicalize.rs` with the pinned toolchain, boots
the pagh-riscv kernel from a checkout's own ``tools/run.py`` (the kernel
repository is never modified) with an ext2 image built by ``mke2fs -d``
containing the example as ``/canonicalize`` (the guest sees
``/mnt/canonicalize``), and checks the serial transcript line by line. The
transcript is copied to ``docs/live-canonicalize.txt``.

The example builds a small tree of directories, files and symlinks on the
writable ext2 mount and checks the realpath values, the documented errors
(ENOENT/ELOOP/ENOTDIR) and the `Path` component helpers, so the run needs no
host peers and no shell fixtures.

Usage::

    python3 tools/live-canonicalize.py

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
EXAMPLE = "examples/canonicalize"
TARGET = "riscv64gc-unknown-none-elf"
TIMEOUT = 180.0

# Every marker the transcript must contain. The example is deterministic (no
# peers, no races), so each line is required exactly.
EXPECTED = (
    "canonicalize: io::stdout() handle writes",
    "canonicalize: tree ready under /mnt/canon-test",
    "canonicalize: root -> /",
    "canonicalize: dot-dot-lexical -> /mnt/canon-test/sub/file.txt",
    "canonicalize: dots-and-slashes -> /mnt/canon-test/sub/file.txt",
    "canonicalize: relative-link -> /mnt/canon-test/sub/file.txt",
    "canonicalize: absolute-link -> /mnt/canon-test/sub/file.txt",
    "canonicalize: symlink-then-dot-dot -> /mnt/canon-test/sub/file.txt",
    "canonicalize: link-dot-dot-link -> /mnt/canon-test/sub/file.txt",
    "canonicalize: directory -> /mnt/canon-test",
    "canonicalize: missing-component -> No such file or directory (expected)",
    "canonicalize: broken-link -> No such file or directory (expected)",
    "canonicalize: symlink-loop -> Too many levels of symbolic links (expected)",
    "canonicalize: file-as-directory -> Not a directory (expected)",
    "canonicalize: Path::canonicalize-method ok",
    "canonicalize: ok (realpath values, errors, Path helpers, io::stdout)",
    "run: /mnt/canonicalize exited with code 0",
)

# No alternative spellings are needed for this deterministic run.
ANY_OF = ()


def die(message: str, code: int = 2) -> None:
    print(f"live-canonicalize: {message}", file=sys.stderr)
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
        default=str(ROOT / "docs" / "live-canonicalize.txt"),
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

    work = pathlib.Path(tempfile.mkdtemp(prefix="pagh-live-canonicalize-"))
    try:
        print("live-canonicalize: building examples/canonicalize.rs", file=sys.stderr)
        build = subprocess.run(
            ["cargo", "build", "--release", "--example", "canonicalize", "--target", TARGET],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if build.returncode != 0:
            print(build.stdout, file=sys.stderr)
            print(build.stderr, file=sys.stderr)
            die("cargo build failed")

        example = ROOT / "target" / TARGET / "release" / "examples" / "canonicalize"
        if not example.is_file():
            die(f"{example} was not built")

        diskroot = work / "root"
        diskroot.mkdir()
        shutil.copy(example, diskroot / "canonicalize")

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
            "run /mnt/canonicalize\\n",
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
        print("live-canonicalize: booting the kernel", file=sys.stderr)
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
                "# image: mke2fs -t ext2 -b 1024 -d <root with target/.../examples/canonicalize as canonicalize> disk.img",
                "# guest command: run /mnt/canonicalize",
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
                print(f"live-canonicalize: MISSING: {marker}", file=sys.stderr)
            for group in missing_groups:
                print(f"live-canonicalize: MISSING one of: {group}", file=sys.stderr)
            print(f"live-canonicalize: transcript is {args.transcript}", file=sys.stderr)
            return 1
        print(f"live-canonicalize: PASS (transcript {args.transcript})", file=sys.stderr)
        return 0
    finally:
        if args.keep_workdir:
            print(f"live-canonicalize: workdir kept at {work}", file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())