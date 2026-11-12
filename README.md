# pagh-api — `libpagh`, userland API для pagh-riscv

`libpagh` — единственный слой между программой и ядром
[`pagh-riscv`](../pagh-riscv). Не libc, не Linux-совместимость и не обёртка
«как получилось»: это `#![no_std]`-крейт без внешних зависимостей, который
реализует ровно контракт из [`docs/abi.md`](docs/abi.md) — таблицу syscall'ов,
структуры, раскладку начального стека, ошибки и рантайм.

```
docs/abi.md          замороженный контракт (источник истины)
pagh-user.ld         раскладка ELF: база 0x10000, _start в .text.entry, R+X / R+W
libpagh/src/sys.rs   60 ecall-обёрток + номера/флаги
libpagh/src/errno.rs Errno (42 значения из ABI) и Result<T>
libpagh/src/abi.rs   Stat/Timespec/Uname/EntryKind, проверка размеров
libpagh/src/rt.rs    _start, разбор argc/argv/envp, паника
libpagh/src/alloc_impl.rs  bump-аллокатор на brk
libpagh/src/io.rs    Read/Write/Seek, Stdin/Stdout/Stderr
libpagh/src/fs.rs    File, Dir/DirEntry, stat, read_link
libpagh/src/net.rs   Ipv4, TcpStream (один активный TCP-клиент), TcpListener (bind/listen/accept) и UdpSocket (bind/sendto/recvfrom)
libpagh/src/tls.rs   TlsStream: TLS 1.3 на подключённом TcpStream (рукопожатие — ядро, fail-closed)
libpagh/src/random.rs getrandom: пул энтропии ядра (fill/u64), короткие чтения скрыты
libpagh/src/dns.rs   резолвер A-записей: один UDP-запрос (без DNSSEC/CNAME-цепочек)
libpagh/src/process.rs Command-lite: fork/execve/wait4, ExitStatus (без пайпов и kill)
libpagh/src/path.rs  normalize/resolve/join, file_name/extension/parent/starts_with, Path
libpagh/src/env.rs   args(), var(), current_dir()
libpagh/src/print.rs print!/println!/eprintln!
examples/hello.rs    пример программы
examples/http_get.rs HTTP/1.0 GET на 10.0.2.2:8000 (нативный TCP-сокет)
examples/udp_echo.rs UDP bind/sendto/recvfrom на 10.0.2.2:9000 (нативный UDP-сокет)
examples/tcp_echo.rs TCP-сервер: bind 0.0.0.0, listen(4), accept одного host-подключения, echo
examples/mutex.rs    четыре потока под futex Mutex (counter=1000)
examples/https_get.rs HTTPS/1.1 GET: dns::resolve -> TcpStream -> TlsStream (1100) -> тело
examples/fs_ls.rs    read_dir + read_to_string + write на /mnt, самопроверка (exit 0)
examples/spawn.rs    Command-lite: self-exec, env/env_clear, exit codes, try_wait/wait
examples/canonicalize.rs fs::canonicalize + Path helpers на реальных symlink'ах /mnt (exit 0)
docs/live-spawn.txt  живой QEMU-транскрипт examples/spawn (харнесс tools/live-spawn.py)
docs/live-canonicalize.txt живой QEMU-транскрипт examples/canonicalize (tools/live-canonicalize.py)
tools/check_abi.py   сверка docs/abi.md с sys.rs/errno.rs/abi.rs (+ §4 fs/process/path/io)
```

## Сборка

Тулчейн закреплён в [`rust-toolchain.toml`](rust-toolchain.toml) — тот же
`nightly-2026-07-12`, что и у ядра; `rustup` сам подхватит его в этом каталоге.
Цель и линкер заданы в [`.cargo/config.toml`](.cargo/config.toml), так что
достаточно:

```sh
cargo build --release --target riscv64gc-unknown-none-elf   # rlib libpagh
./build.sh                                                   # ELF examples/hello
rustup run nightly-2026-07-12 cargo build --release \
    --example http_get --target riscv64gc-unknown-none-elf   # ELF examples/http_get
rustup run nightly-2026-07-12 cargo build --release \
    --example udp_echo --target riscv64gc-unknown-none-elf   # ELF examples/udp_echo
rustup run nightly-2026-07-12 cargo build --release \
    --example tcp_echo --target riscv64gc-unknown-none-elf   # ELF examples/tcp_echo
rustup run nightly-2026-07-12 cargo build --release \
    --example https_get --target riscv64gc-unknown-none-elf  # ELF examples/https_get
rustup run nightly-2026-07-12 cargo build --release \
    --example fs_ls --target riscv64gc-unknown-none-elf      # ELF examples/fs_ls
rustup run nightly-2026-07-12 cargo build --release \
    --example spawn --target riscv64gc-unknown-none-elf      # ELF examples/spawn
rustup run nightly-2026-07-12 cargo build --release \
    --example canonicalize --target riscv64gc-unknown-none-elf  # ELF examples/canonicalize
python3 tools/check_abi.py                                   # дрейф ABI
```

`build.sh` печатает точную команду сборки и путь к статическому ELF
(`target/riscv64gc-unknown-none-elf/release/examples/hello`), который ядро
запускает через `run <path>`.

`examples/http_get.rs` — живой сокетный пример: `TcpStream::connect` к
`10.0.2.2:8000` (QEMU user network), HTTP/1.0 `GET /`, печать ответа до EOF.
Его прогоняет `tools/test_sockets.py` из репозитория ядра; адрес — литерал,
потому что DNS в ABI нет. `examples/udp_echo.rs` — UDP-пара: `UdpSocket::bind`
на локальный порт, `send_to` на `10.0.2.2:9000`, блокирующий `recv_from`;
живой прогон — `tools/test_udp.py` ядра (host-эхо + ext2-образ).
`examples/tcp_echo.rs` — TCP-сервер: `TcpListener::bind_with_backlog` на
`0.0.0.0:45679` (backlog 4), один `accept`, чтение до EOF и echo; живой прогон —
`tools/test_tcp_server.py` ядра (`mke2fs -d` + QEMU `hostfwd`).

`examples/https_get.rs` — живой HTTPS/1.1-клиент: `dns::resolve` (A-запрос по
UDP), `TcpStream::connect(ip, 443)`, `TlsStream::connect(socket, host)` —
рукопожатие делает ядро — затем `GET` и разбор ответа (`Content-Length`,
`chunked` или тело до закрытия); живой прогон — фаза userland в
`tools/test_tls.py` ядра (образ `mke2fs -d`, `run /mnt/https_get example.com /`,
транскрипт `docs/live-tls.txt`).

`pagh-user.ld` повторяет `pagh-riscv/userland/user.ld`: образ начинается с
`0x1_0000`, `_start` лежит первым в `.text.entry`, `.text`/`.rodata` дают
R+X/R-only PT_LOAD, `.data`/`.bss` — R+W. `_start` находится по адресу
`0x10000`, что проверяется `readelf -h` (см. отчёт в коммите).

## Контракт входа

Ядро загружает статический RV64 ELF, кладёт на стек `argc/argv/envp` и входит
в U-mode:

```text
a0 = argc
a1 = argv
sp -> [argc][argv0 … argvN-1][NULL][env0 … envK-1][NULL][строки]
```

`_start` (global_asm в `rt.rs`) сохраняет `sp`, вызывает `libpagh::start(sp)`,
тот разбирает стек, вызывает

```rust
extern "C" fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32
```

и делает `exit(code)`. Возврата из `_start` нет. `argv[0]` — путь, набранный в
шелле; окружение содержит как минимум `PWD`, относительные пути программа
резолвит сама (`path::resolve`), потому что путь в syscall'ах ядра абсолютный.

## Как выглядит программа

```rust
#![no_std]
#![no_main]

use libpagh::println;

#[no_mangle]
pub extern "C" fn main(argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    println!("hello, argc = {argc}");
    0
}
```

Важно: программа обязана **использовать** `libpagh` (здесь — `println!`).
Иначе rustc не подключает rlib, а вместе с ним не приходят `_start` и
panic-handler. Зависимость — обычная path-зависимость:

```toml
[dependencies]
libpagh = { path = "../pagh-api/libpagh" }
```

## Что есть в API

| Модуль | Что даёт |
|---|---|
| `sys` | `read write openat close lseek fstat getdents64 brk clock_gettime nanosleep uname readlinkat getpid sched_yield exit` + `mkdirat unlinkat symlinkat linkat renameat2 fchmodat ftruncate` + `socket bind listen accept connect sendto recvfrom send recv shutdown` + `getrandom` + `tls_connect tls_write tls_read tls_close`; все, кроме `exit`, возвращают `Result<T, Errno>`; незнакомый ядру номер даёт `ENOSYS` |
| `abi` | `Stat` (24 B), `Timespec` (16 B), `Uname` (80 B), `SockAddrIn` (16 B), `PollFd` (8 B), `SigAction` (32 B), `SigAltStack` (24 B), `SigInfo` (128 B), `MContext` (264 B), `UContext` (288 B), `EntryKind`, `Dirent`-константы; размеры проверяются `const assert` |
| `errno` | 42 errno из ABI + `Errno::Unknown` для дрейфа; `Display` с текстами для CLI (`EPROTO` — сломанное рукопожатие, `EKEYREJECTED` — отклонённый сертификат) |
| `io` | `Read`/`Write`/`Seek`, `Stdin`/`Stdout`/`Stderr` и конструкторы `stdin()`/`stdout()`/`stderr()`, `copy`, `SeekFrom` |
| `net` | `Ipv4` (литерал и `Ipv4::parse`), `TcpStream::connect(host, port)`, `Read`/`Write` (неблокирующий `read`: `EAGAIN` → повтор), `read_to_end`, `shutdown`; `TcpListener::bind`/`bind_to`/`bind_with_backlog`/`bind_nonblocking` и `accept` (peer `SockAddrIn`); `UdpSocket::bind`/`bind_to`, `send_to`, `recv_from`/`recv_from_nonblocking`; только IPv4-литералы, один listener + принятые соединения, UDP ≤ 1024 B |
| `tls` | `TlsStream::connect(TcpStream, host)` — рукопожатие TLS 1.3 **делает ядро** (цепочка до CA-бандла, SAN, срок, `CertificateVerify`; флага «не проверять» нет), `Read`/`Write` открытым текстом, `read_to_end`, `shutdown`, `close` (шлёт `close_notify`; `Drop` закрывает без него) |
| `random` | `fill(&mut [u8])` (сам обходит 64 KiB-потолок вызова), `u64()`; без seed'а — `EAGAIN`, а не предсказуемые байты |
| `dns` | `resolve(host, server)` — один A-запрос по UDP (`DEFAULT_SERVER = 10.0.2.3` в QEMU user network); без DNSSEC, без TCP-retry, CNAME-цепочка без A-записи — `EPROTO` |
| `fs` | `File::open/create/open_with`, `File::set_len`, `read_to_end`, `read_to_string`, `write`/`write_all`, `create_dir`/`create_dir_all`, `remove_file`/`remove_dir`, `rename`, `hard_link`, `symlink`, `truncate` (по пути), `set_permissions`, `stat`, `read_link`, `Dir::open`; `read_dir` — std-образный итератор `DirEntry` поверх `getdents64` с `file_name`/`path`/`file_type`/`metadata` |
| `path` | `normalize`, `resolve` (относительно `PWD`), `join`, `basename`, `dirname`; std-образные `file_name`/`extension`/`parent`/`starts_with`; `Path` (+ `Path::canonicalize` через `fs::canonicalize`) |
| `process` | `Command::new`, `arg`/`args`, `env`/`env_clear`, `spawn`, `status`; `Child::id`/`wait`/`try_wait`; `ExitStatus::code`/`signal`/`success` — lite `std::process::Command` поверх `fork`+`execve`+`wait4`. Без пайпов/PATH-поиска и без cross-process `kill` (kill в ABI адресует только текущий run); ошибка `execve` в ребёнке → сообщение на fd 2 и exit 127 |
| `env` | `args()` (включая `argv[0]`), `vars()`, `var("PWD")`, `current_dir()`, `program_name()` |
| `alloc` | bump-аллокатор на `brk`: работают `Vec`, `String`, `format!` |
| `print` | `print! println! eprint! eprintln!` |

Ошибки не паникуют: `main` сам решает, что печатать и с каким кодом выходить.
`EAGAIN` не ретраится внутри `Read`: это сигнал «на fd 0 пока нет байта»
(и «на сокете пока нет данных»), и прикладной код делает `sched_yield` и
повторяет (см. ниже). Исключение — `TcpStream::read_to_end`: он сам крутит
этот цикл до EOF. `TlsStream::read` **блокирующий** (бюджет простоя в ядре,
~20 s): `Ok(0)` — `close_notify`, а обрыв TCP без него — `ECONNRESET`, поэтому
обрезанное тело нельзя принять за дочитанное.

## Память

`brk`-аллокатор страничный: первый `brk(0)` узнаёт текущую границу, дальше
кусок не меньше 64 KiB, выравнивание по 4 KiB, `brk` только растёт. `dealloc`
— no-op: память живёт до `exit`. Это осознанный компромисс для одноразового
CLI-процесса, а не универсальный malloc.

Если ядро ещё не реализовало `brk`, первый же `Vec`/`String`/`format!`
приводит к `program: out of memory (brk failed); …` на fd 2 и `exit(102)`.
Это не паника и не зависание.

## stdin, EOF и `poll`

`read(0)` по ABI возвращает `-EAGAIN`, пока в очереди нет байта; `poll(7)`
(0.12.0) умеет ждать `POLLIN` на консоли, поэтому новые читатели могут спать
вместо `read` + `sched_yield`. Старые applets оставлены на yield-цикле —
поведение то же. `0` байт считается EOF, но консольный fd 0 в текущем ядре
EOF не выдаёт — `cat` без файлов будет ждать ввод вечно, как интерактивный
`cat`. Это ограничение ядра, записано в отчёт.

## Паника и аварийные коды

* паника: `program: panic at <file>:<line>:<col>` в fd 2 → `exit(101)`; без
  размотки, без backtrace (у ядра пока нет потребителя DWARF);
* нехватка памяти: `program: out of memory (brk failed); requested N bytes at
  alignment A` → `exit(102)`;
* `Errno::Unknown` — признак расхождения ядра и `docs/abi.md`; это не
  нормальная ошибка, а сигнал вызвать Lead'а.

## Чего нет

Ни динамического линкера, ни `auxv`, ни libc, ни `std`. Процессы есть:
`fork`/`execve`/`wait4`/`getppid` (0.16.0). Потоки и futex есть (0.13.0: [`thread::Builder`](libpagh/src/thread.rs)), [`sync::Mutex`/`RwLock`](libpagh/src/sync.rs) — 0.5.0,
сигналы — `kill`/`tgkill`/`rt_sigaction`/`rt_sigprocmask`/`rt_sigreturn`
(0.14.0), память — `mmap`/`munmap`/`mprotect`, дескрипторы — `pipe2`/`dup`/
`dup2`/`poll`/`ppoll`. Файловые мутации есть:
`mkdirat`/`unlinkat`/`symlinkat`/`linkat`/`renameat2`/`fchmodat`/`ftruncate`
(номера 34–37, 53, 46, 276) и обёртки `fs::create_dir`, `remove_file`,
`remove_dir`, `rename`, `hard_link`, `symlink`, `truncate`, `set_permissions`,
`File::set_len`. Чего по-прежнему нет: `chown`, `utimes`, `O_EXCL`/`O_DIRECTORY`,
`AT_SYMLINK_NOFOLLOW`, жёстких ссылок на каталог и **долговременного** `chmod` —
режим живёт только до конца прогона (см. ниже).

Сокеты: один **активный** TCP-клиент (`AF_INET` + `SOCK_STREAM`, `connect` по
IPv4-литералу, `send`/`recv`, `shutdown`), TCP-сервер (`bind` + `listen(201)` +
`accept(202)`, один listener, очередь ≤ 4, принятые соединения — отдельные
дескрипторы) и UDP (`AF_INET` + `SOCK_DGRAM`, `bind`, `sendto`,
блокирующий/неблокирующий `recvfrom`). Нет UDP-`connect`, IPv6,
`sendmsg`/`recvmsg`, `getaddrinfo`-подобного резолвера (есть `dns::resolve` —
один A-запрос) и **TLS-сервера**; второе **активное**
TCP-соединение одновременно невозможно.

TLS есть как **клиент** и только на уже подключённом сокете (0.6.1):
`TlsStream::connect(TcpStream, host)` — рукопожатие выполняет ядро тем же
fail-closed проверяющим слоем, что и shell'овый `https` (цепочка до
закоммиченного CA-бандла, SAN, срок, `CertificateVerify`); `flags`-флага
«не проверять сертификат» не существует ни в ABI, ни здесь. Что остаётся
программе: резолв (`dns`), TCP-`connect`, HTTP-framing (см.
`examples/https_get.rs`) и решение, что слать. Одновременно живых TLS-сессий —
**4** на ядро, `read` блокирующий (бюджет ~20 s), `tls_read` отдаёт до 4096 B
за вызов, а обрыв TCP без `close_notify` — `ECONNRESET`, не EOF: обрезанный
ответ нельзя принять за дочитанный. `dup`/`fork` сессию не копируют
(`-ENOTSUP`), TLS-сервера, session resumption, ALPN, клиентских сертификатов,
TLS 1.2 и ниже нет.

`random::fill` — пул энтропии ядра (`/chosen/rng-seed` + SHA-256 ratchet); без
seed'а ядро отвечает `EAGAIN`, а не предсказуемыми байтами, и это надо
обрабатывать как фатальное для ключей. `poll` на socket-fd работает
(готовность listener'а к `accept`, потока к чтению/записи). TCP `recv`
неблокирующий (`EAGAIN` — повторить), `shutdown` закрывает оба направления;
UDP `recvfrom` ждёт до дейтаграммы или ~10 s,
`MSG_DONTWAIT`/`SOCK_NONBLOCK` даёт `EAGAIN`. `setsockopt` для типовых
`SO_*`/`TCP_NODELAY` принимается и игнорируется.

## Осознанные решения и открытые вопросы ABI

Всё, что неоднозначно в `docs/abi.md`, решено консервативно и перечислено
здесь; это же — список для Lead'а:

1. **`dirfd` игнорируется.** Передаём `AT_FDCWD = -100` (Linux-значение), хотя
   ABI его не описывает; важно только то, что ядро его игнорирует.
2. **`stat` = `openat(O_RDONLY)` + `fstat` + `close`.** Отдельного `statat` в
   ABI нет, поэтому файл с mode `000` нельзя `stat`'ить, если нельзя открыть.
   Это ограничение ABI, а не реализации.
3. **`openat` на каталог.** ABI не говорит, разрешён ли `O_RDONLY`-open
   каталога, но без этого `getdents64` бесполезен. `Dir::open` на это
   рассчитывает; если ядро откажет — нужен `O_DIRECTORY` или отдельный
   syscall.
4. **`nanosleep`.** Второй аргумент (`rem`) ABI игнорирует; передаём указатель
   на занулённый `Timespec`, а не NULL, чтобы ядро, валидирующее оба
   указателя, не вернуло `EFAULT`.
5. **`readlinkat`.** Длина не определена; начинаем с 256 B и удваиваем до
   4096 B, дальше `ENAMETOOLONG`. Цель декодируется lossy (имена не обязаны
   быть UTF-8).
6. **`PWD`.** Если переменной нет или она не абсолютная — считаем cwd `/`.
   Контракт обещает `PWD`, но программа не должна падать от кривого окружения.
7. **Начальный `brk`.** ABI не фиксирует, где он находится после загрузки;
   аллокатор вызывает `brk(0)` и доверяет ответу. Ядру нужно вернуть границу
   после последнего PT_LOAD, иначе первый `Vec` затрёт `.bss`/кучу.
8. **`Dirent`.** Заголовок записи — 12 байт (`u64 ino`, `u8 kind`, `u8
   name_len`, `u16 reserved`), запись выровнена на 8. Это не Linux-ный
   `struct linux_dirent64`; парсер в `fs.rs` — тому доказательство.
9. **UTF-8.** argv/envp/имена lossy-декодируются, а `read_to_string` при
   невалидном UTF-8 возвращает `EINVAL` (в ABI нет `EILSEQ`).
10. **`clock_gettime`.** 0 — realtime/uptime, 1 — monotonic, как в таблице;
    `Timespec` — 16 байт, `nanos` в диапазоне 0..1e9 (ядро обязано
    нормализовать).
11. **Длина пути.** ABI не задаёт `PATH_MAX`, а ядро принимает путь не
    длиннее 255 байт + NUL (`PATH_CAPACITY = 256`). `path::resolve` длинных
    путей не обрезает: syscall вернёт `ENAMETOOLONG`, программа напечатает
    ошибку. Это ограничение ядра, а не скрытый truncate.
12. **`mode` при `O_CREAT` и `mkdirat`.** ABI говорит, что mode применяется;
    текущее ядро его игнорирует (ext2 создаёт 0o644/0o755): в VFS нет
    `set_mode`. `File::create` передаёт `0o666` и не полагается на права.
13. **`chmod` не сохраняется.** `fchmodat` записывает `mode & 0o7777` в таблицу
    прогона, и `fstat` показывает новые биты до конца программы; в inode они не
    пишутся, права ни на что не влияют, следующий процесс видит диск снова.
    Это задокументированный компромисс (в VFS ядра нет `set_mode`), а не
    обещание persistence. На синтетическом узле (корень `/`) — `ENOTSUP`.
14. **`unlinkat` и `linkat` не разыменовывают последнюю ссылку.** `rm link`
    удаляет саму ссылку, а `linkat` создаёт жёсткую ссылку на неё; `fchmodat`,
    наоборот, идёт по ссылке, как `chmod`. `AT_SYMLINK_NOFOLLOW` в ABI нет:
    ненулевые неизвестные флаги — `EINVAL`, а не тихий игнор.

15. **Сокеты — один активный TCP-клиент, TCP-сервер и UDP.** `connect`
    принимает только `sockaddr_in` (16 байт) с IPv4-литералом; DNS-а нет. TCP
    `recv` неблокирующий по контракту `read(0)`: `EAGAIN` — это «пока нет
    данных», 0 — EOF. `shutdown` не имеет half-close: оба направления
    закрываются, дескриптор остаётся открытым (чтение → 0, запись → `EPIPE`)
    до `close`. Сервер: `bind` резервирует порт, `listen(201)` включает
    пассивный приём (backlog ≤ 4), `accept(202)` отдаёт соединение и peer-адрес;
    у ядра один listener, восемь слотов соединений (клиент + принятые) и 30 s
    бюджет блокирующего `accept` (`SOCK_NONBLOCK` → сразу `EAGAIN`). Второй
    **активный** TCP `socket()` при открытом первом — `ENOSPC`; UDP-сокетов
    можно много, `bind` даёт порт, `recvfrom` без `MSG_DONTWAIT` блокируется.
    Это записано в `docs/abi.md`.
16. **Номера `setsockopt`/`getsockopt`.** В generic RISC-V таблице это 208/209;
    200 — `bind`, 201 — `listen`, 202 — `accept`; 203 — `connect`.
    `tools/check_abi.py` сверяет таблицу с `sys.rs`, так что дрейф номеров не
    прошёл бы проверку.

## Статус ядра

Проверено на `pagh-riscv` **0.21.0** (`adae144`, main на 2026-11-09): ядро
отвечает на все 60 syscall'ов из `docs/abi.md` §2 — сверка таблицы контракта с
константами `pagh-riscv/src/user/syscall.rs` не находит ни одного
недостающего, лишнего или переименованного номера (60 = 60). Все семейства из
«Что есть в API» есть в ядре этого среза: файлы и мутации, сокеты, потоки и
futex, сигналы, процессы, энтропия и TLS; `getrandom` (278) и четыре TLS-вызова
(1100–1103) — последнее добавление, 0.21.0.

Контракт проверяется с двух сторон. Встроенная фикстура ядра
`pagh-riscv/userland/abi.rs` покрывает тот же набор без libpagh, а живые
сценарии `pagh-riscv/tools/test_sockets.py`, `test_udp.py`,
`test_tcp_server.py` и `test_tls.py` строят ext2-образ из примеров этого
репозитория (`http_get`, `udp_echo`, `tcp_echo`, `https_get`), запускают QEMU
user net и складывают транскрипты в `pagh-riscv/docs/live-*.txt`.

CI (`.github/workflows/ci.yml`) на каждый push и PR ставит закреплённый
`nightly-2026-07-12`, собирает `libpagh` и примеры и запускает
`tools/check_abi.py`; дрейф номеров, errno, флагов или размеров структур
краснит сборку.

## Тест на дрейф ABI

```sh
python3 tools/check_abi.py
# ABI check OK: 60 syscalls, 42 errnos, 7 open flags, 10 struct sizes, 26 fs items, 13 process items, 10 path items, 9 io items
```

Скрипт читает `docs/abi.md`, парсит таблицу syscall'ов, секцию ошибок, флаги
`O_*` и комментарии размеров структур, затем грепает `sys.rs`, `errno.rs`,
`abi.rs`. Проверяет и обратное направление: лишний `SYS_*`/`O_*`/errno без
строки в контракте — тоже ошибка. Отдельно сверяются указатели §4 `fs` → `fs.rs`, `process` → `process.rs`,
`path` → `path.rs` и `io` → `io.rs`: имя, которое контракт обещает, но
которого нет в коде, краснит проверку.