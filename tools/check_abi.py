#!/usr/bin/env python3
"""Check that `docs/abi.md` and the Rust definitions in `libpagh` agree.

The ABI document is the frozen contract. This script is the drift alarm that
runs without a network or a cross toolchain:

  * every syscall row in the doc has a `SYS_*` constant with the same number,
    and no `SYS_*` constant exists that the doc does not list;
  * every errno in doc §3 has an `Errno` variant with the same discriminant,
    and no variant (other than `Unknown`, the drift marker) is undocumented;
  * every `O_*`/`AT_*` flag in doc §2 matches `libpagh/src/sys.rs`;
  * the documented `Stat`/`Timespec`/`Uname`/`SockAddrIn` sizes match the compile-time
    assertions in `libpagh/src/abi.rs`.

Exit code 0 on agreement, 1 on any drift, 2 when a file/pattern is missing
(which means the contract or the parser moved and a human must look).
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ABI_MD = ROOT / "docs" / "abi.md"
SYS_RS = ROOT / "libpagh" / "src" / "sys.rs"
ERRNO_RS = ROOT / "libpagh" / "src" / "errno.rs"
ABI_RS = ROOT / "libpagh" / "src" / "abi.rs"


class Drift(Exception):
    """A contract/implementation mismatch, collected and reported together."""


def read(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"check_abi: cannot read {path}: {error}")


def section(text: str, heading: str) -> str:
    """Return the text between `heading` and the next `## ` heading."""
    start = text.find(heading)
    if start < 0:
        raise SystemExit(f"check_abi: section {heading!r} not found")
    end = text.find("\n## ", start + len(heading))
    return text[start:] if end < 0 else text[start:end]


def documented_syscalls(doc: str) -> dict[str, int]:
    found: dict[str, int] = {}
    for line in doc.splitlines():
        match = re.match(r"\|\s*(\d+)\s*\|\s*`([a-z0-9_]+)`\s*\|", line)
        if match:
            number = int(match.group(1))
            name = match.group(2)
            if name in found:
                raise SystemExit(f"check_abi: duplicate syscall row {name}")
            found[name] = number
    if not found:
        raise SystemExit("check_abi: no syscall rows parsed from docs/abi.md")
    return found


def documented_errnos(doc: str) -> dict[str, int]:
    errors = section(doc, "## 3. Errors")
    found = {name: int(number) for name, number in re.findall(r"([A-Z][A-Z0-9_]*)=(\d+)", errors)}
    if not found:
        raise SystemExit("check_abi: no errno constants parsed from docs/abi.md")
    return found


def documented_flags(doc: str) -> dict[str, int]:
    text = doc
    start = text.find("Flags:")
    if start < 0:
        raise SystemExit("check_abi: `Flags:` line not found in docs/abi.md")
    # The paragraph may wrap over several lines; stop at the first blank line.
    end = text.find("\n\n", start)
    paragraph = text[start:] if end < 0 else text[start:end]
    found: dict[str, int] = {}
    for name, value in re.findall(
        r"`((?:O|AT)_[A-Z0-9_]+)=(0x[0-9A-Fa-f]+|\d+)`", paragraph
    ):
        found[name] = int(value, 0)
    if not found:
        raise SystemExit("check_abi: no O_* flags parsed from docs/abi.md")
    return found


def rust_syscalls(source: str) -> dict[str, int]:
    return {
        name: int(number)
        for name, number in re.findall(
            r"pub const SYS_([A-Z0-9_]+): usize = (\d+);", source
        )
    }


def rust_errnos(source: str) -> dict[str, int]:
    # Enum variants are `NAME = 7,`; match only inside the `Errno` enum to avoid
    # catching match arms or `const` declarations elsewhere in the file.
    enum = re.search(r"pub enum Errno\s*\{(?P<body>.*?)\n\}", source, re.S)
    if not enum:
        raise SystemExit("check_abi: `pub enum Errno` not found in errno.rs")
    return {
        name: int(number)
        for name, number in re.findall(
            r"^\s*([A-Z][A-Z0-9_]*)\s*=\s*(\d+),", enum.group("body"), re.M
        )
    }


def rust_flags(source: str) -> dict[str, int]:
    return {
        name: int(number, 0)
        for name, number in re.findall(
            r"pub const ((?:O|AT)_[A-Z0-9_]+): u32 = (0x[0-9A-Fa-f]+|\d+);", source
        )
    }


def rust_struct_sizes(source: str) -> dict[str, int]:
    # Only the ABI structs; primitive-width asserts are not ABI struct sizes.
    wanted = {"Stat", "Timespec", "Uname", "SockAddrIn"}
    return {
        name: int(size)
        for name, size in re.findall(
            r"size_of::<(\w+)>\(\) == (\d+)", source
        )
        if name in wanted
    }


def documented_struct_sizes(doc: str) -> dict[str, int]:
    sizes: dict[str, int] = {}
    for name in ("Stat", "Timespec", "Uname", "SockAddrIn"):
        match = re.search(
            rf"struct\s+{name}\b.*?//\s*(\d+)\s+bytes", doc, re.S
        )
        if not match:
            raise SystemExit(f"check_abi: size comment for struct {name} not found")
        sizes[name] = int(match.group(1))
    return sizes


def compare(label: str, doc: dict[str, int], rust: dict[str, int], errors: list[str]) -> None:
    for name in sorted(set(doc) | set(rust)):
        if name not in rust:
            errors.append(f"{label}: docs list {name}={doc[name]} but Rust has no such item")
        elif name not in doc:
            errors.append(f"{label}: Rust defines {name}={rust[name]} but docs do not list it")
        elif doc[name] != rust[name]:
            errors.append(f"{label}: {name} is {doc[name]} in docs but {rust[name]} in Rust")


def main() -> int:
    doc = read(ABI_MD)
    errors: list[str] = []

    doc_sys = documented_syscalls(doc)
    rust_sys = rust_syscalls(read(SYS_RS))
    # The doc names syscalls in lowercase (`getdents64`); the constants are
    # uppercase (`SYS_GETDENTS64`).
    doc_sys_upper = {name.upper(): number for name, number in doc_sys.items()}
    compare("syscall", doc_sys_upper, rust_sys, errors)

    doc_errno = documented_errnos(doc)
    errno_impl = rust_errnos(read(ERRNO_RS))
    # `Unknown` is the deliberate out-of-table marker, not an ABI number.
    errno_impl.pop("Unknown", None)
    compare("errno", doc_errno, errno_impl, errors)

    doc_flags = documented_flags(doc)
    flags_impl = rust_flags(read(SYS_RS))
    compare("open flag", doc_flags, flags_impl, errors)

    doc_sizes = documented_struct_sizes(doc)
    sizes_impl = rust_struct_sizes(read(ABI_RS))
    compare("struct size", doc_sizes, sizes_impl, errors)

    # Sanity: the numbers above are only useful if the table is the real one.
    expected = {
        "read": 63,
        "write": 64,
        "openat": 56,
        "close": 57,
        "lseek": 62,
        "fstat": 80,
        "getdents64": 61,
        "brk": 214,
        "clock_gettime": 113,
        "nanosleep": 101,
        "uname": 160,
        "readlinkat": 78,
        "getpid": 172,
        "sched_yield": 124,
        "exit": 93,
        # The file-mutation family (native-mut). Additive to the first-port
        # table above; docs, libpagh and the kernel dispatch share these
        # numbers.
        "mkdirat": 34,
        "unlinkat": 35,
        "symlinkat": 36,
        "linkat": 37,
        "fchmodat": 53,
        "ftruncate": 46,
        "renameat2": 276,
        # The BSD-socket subset (native-sock). Additive as well: one connected
        # AF_INET/SOCK_STREAM socket, no listener, UDP or DNS.
        "socket": 198,
        "connect": 203,
        "sendto": 206,
        "recvfrom": 207,
        "setsockopt": 208,
        "getsockopt": 209,
        "shutdown": 210,
        # The POSIX memory/descriptor subset (native-posix).
        "mmap": 222,
        "munmap": 215,
        "mprotect": 226,
        "pipe2": 59,
        "dup": 23,
        "dup2": 24,
        "poll": 7,
        "ppoll": 72,
    }
    if doc_sys != expected:
        errors.append(
            "syscall table changed: docs/abi.md no longer matches the frozen "
            f"first-port table ({len(expected)} numbers)"
        )

    if errors:
        print("ABI DRIFT:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    print(
        "ABI check OK: "
        f"{len(doc_sys)} syscalls, {len(doc_errno)} errnos, "
        f"{len(doc_flags)} open flags, {len(doc_sizes)} struct sizes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())