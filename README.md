# pagh-api — `libpagh`, userland API для pagh-riscv

`libpagh` — единственный слой между программой и ядром
[`pagh-riscv`](../pagh-riscv). Не libc, не Linux-совместимость и не обёртка
«как получилось»: это `#![no_std]`-крейт без внешних зависимостей, который
реализует ровно контракт из [`docs/abi.md`](docs/abi.md) — таблицу syscall'ов,
структуры, раскладку начального стека, ошибки и рантайм.

```
docs/abi.md          замороженный контракт (источник истины)
pagh-user.ld         раскладка ELF: база 0x10000, _start в .text.entry, R+X / R+W
libpagh/src/sys.rs   15 ecall-обёрток + номера/флаги
libpagh/src/errno.rs Errno (19 значений из ABI) и Result<T>
libpagh/src/abi.rs   Stat/Timespec/Uname/EntryKind, проверка размеров
libpagh/src/rt.rs    _start, разбор argc/argv/envp, паника
libpagh/src/alloc_impl.rs  bump-аллокатор на brk
libpagh/src/io.rs    Read/Write/Seek, Stdin/Stdout/Stderr
libpagh/src/fs.rs    File, Dir/DirEntry, stat, read_link
libpagh/src/path.rs  normalize/resolve относительно PWD
libpagh/src/env.rs   args(), var(), current_dir()
libpagh/src/print.rs print!/println!/eprintln!
examples/hello.rs    пример программы
tools/check_abi.py   сверка чисел в docs/abi.md с sys.rs/errno.rs/abi.rs
```

## Сборка

Тулчейн закреплён в [`rust-toolchain.toml`](rust-toolchain.toml) — тот же
`nightly-2026-07-12`, что и у ядра; `rustup` сам подхватит его в этом каталоге.
Цель и линкер заданы в [`.cargo/config.toml`](.cargo/config.toml), так что
достаточно:

```sh
cargo build --release --target riscv64gc-unknown-none-elf   # rlib libpagh
./build.sh                                                   # ELF examples/hello
python3 tools/check_abi.py                                   # дрейф ABI
```

`build.sh` печатает точную команду сборки и путь к статическому ELF
(`target/riscv64gc-unknown-none-elf/release/examples/hello`), который ядро
запускает через `run <path>`.

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
| `sys` | `read write openat close lseek fstat getdents64 brk clock_gettime nanosleep uname readlinkat getpid sched_yield exit`; все, кроме `exit`, возвращают `Result<T, Errno>`; незнакомый ядру номер даёт `ENOSYS` |
| `abi` | `Stat` (24 B), `Timespec` (16 B), `Uname` (80 B), `EntryKind`, `Dirent`-константы; размеры проверяются `const assert` |
| `errno` | 19 errno из ABI + `Errno::Unknown` для дрейфа; `Display` с текстами для CLI |
| `io` | `Read`/`Write`/`Seek`, `Stdin`/`Stdout`/`Stderr`, `copy`, `SeekFrom` |
| `fs` | `File::open/create/open_with`, `read_to_end`, `read_to_string`, `write`, `stat`, `read_link`, `Dir`/`DirEntry` поверх `getdents64` |
| `path` | `normalize`, `resolve` (относительно `PWD`), `join`, `basename`, `dirname`, `Path` |
| `env` | `args()` (включая `argv[0]`), `vars()`, `var("PWD")`, `current_dir()`, `program_name()` |
| `alloc` | bump-аллокатор на `brk`: работают `Vec`, `String`, `format!` |
| `print` | `print! println! eprint! eprintln!` |

Ошибки не паникуют: `main` сам решает, что печатать и с каким кодом выходить.
`EAGAIN` не ретраится внутри `Read`: это сигнал «на fd 0 пока нет байта»,
и прикладной код делает `sched_yield` и повторяет (см. ниже).

## Память

`brk`-аллокатор страничный: первый `brk(0)` узнаёт текущую границу, дальше
кусок не меньше 64 KiB, выравнивание по 4 KiB, `brk` только растёт. `dealloc`
— no-op: память живёт до `exit`. Это осознанный компромисс для одноразового
CLI-процесса, а не универсальный malloc.

Если ядро ещё не реализовало `brk`, первый же `Vec`/`String`/`format!`
приводит к `program: out of memory (brk failed); …` на fd 2 и `exit(102)`.
Это не паника и не зависание.

## stdin, EOF и почему нет `poll`

`read(0)` по ABI возвращает `-EAGAIN`, пока в очереди нет байта. Готового
`poll` в ABI нет, поэтому читатели консоли (`cat`, `wc`, `head`, `tail` в
`pagh-programs`) крутят `read` + `sched_yield`. `0` байт считается EOF, но
консольный fd 0 в текущем ядре EOF не выдаёт — `cat` без файлов будет ждать
ввод вечно, как интерактивный `cat`. Это ограничение ядра, записано в отчёт.

## Паника и аварийные коды

* паника: `program: panic at <file>:<line>:<col>` в fd 2 → `exit(101)`; без
  размотки, без backtrace (у ядра пока нет потребителя DWARF);
* нехватка памяти: `program: out of memory (brk failed); requested N bytes at
  alignment A` → `exit(102)`;
* `Errno::Unknown` — признак расхождения ядра и `docs/abi.md`; это не
  нормальная ошибка, а сигнал вызвать Lead'а.

## Чего нет

Ни потоков, ни `fork`, ни сигналов, ни сокетов, ни `mmap`, ни `poll`, ни
динамического линкера, ни `auxv`/TLS, ни libc, ни `std`. Нет и файловых
операций, которых нет в ABI: `rename`, `unlink`, `mkdir`, `chmod` и т. п.
появятся только вместе с syscall'ами ядра.

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
11. **Длина пути.** ABI не задаёт `PATH_MAX`, а `native-abi` принимает не
    длиннее 255 байт + NUL (`PATH_CAPACITY = 256`). `path::resolve` длинных
    путей не обрезает: syscall вернёт `ENAMETOOLONG`, программа напечатает
    ошибку. Это ограничение ядра, а не скрытый truncate.
12. **`mode` при `O_CREAT`.** ABI говорит, что mode применяется; текущее ядро
    его игнорирует (ext2 создаёт 0o644). `File::create` передаёт `0o666` и не
    полагается на права, поэтому поведение не меняется, но обещание ABI
    расходится с реализацией.

## Статус ядра (на момент коммита)

Ветка `native-abi` рабочего дерева Lead'а уже отвечает на все 15 syscall'ов
этого контракта (плюс свой selftest). Но проверка в QEMU на срезе
`1ce872f` + незакоммиченные правки упёрлась в баги **ядра**, не в `libpagh`:

* encoder `getdents64` считает `record_len = 8 + align8(name_len)`, хотя имя
  пишется со смещения 12 (12-байтовый заголовок из ABI); встроенный
  `selftest` падает на `abi: FAIL getdents64 /mnt names (code 70)`;
* чтение ext2 за 12-м прямым блоком (файловое смещение ≥ 48 KiB) **зависает**
  в S-mode без исключения. Поэтому ELF с `.rodata`, пересекающим 48 KiB
  (в частности `paghbox` с зеркальной раскладкой), не загружается; файл на
  221 KiB с маленькими PT_LOAD загружается нормально;
* после завершения user-программы оболочка получает store page fault
  (`stval=0x201000000`, `Arc::clone`) и машина останавливается — воспроизводится
  даже на пробе, вызывающей только `getpid`;
* часть загрузок падает в `src/arch/riscv64/mmu.rs:206` (misaligned
  `read_volatile`) или в загрузчике ELF (`0x1a000 is not mapped`); повторный
  запуск проходит.

Поэтому `libpagh` и `hello` **собираются** в валидный ELF; `paghbox` и все 12
applet'ов успешно отработали на временной 2-segment раскладке (тот же исходник,
`.rodata` сразу после `.text`), но коммитнутая зеркальная `user.ld` ждёт
исправления ext2. Подробности, адреса и логи — в отчёте Lead'у. Это
зафиксировано, а не замаскировано.

## Тест на дрейф ABI

```sh
python3 tools/check_abi.py
# ABI check OK: 15 syscalls, 19 errnos, 6 open flags, 3 struct sizes
```

Скрипт читает `docs/abi.md`, парсит таблицу syscall'ов, секцию ошибок, флаги
`O_*` и комментарии размеров структур, затем грепает `sys.rs`, `errno.rs`,
`abi.rs`. Проверяет и обратное направление: лишний `SYS_*`/`O_*`/errno без
строки в контракте — тоже ошибка.