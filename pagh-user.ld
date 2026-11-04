/* The userland link contract for `libpagh` programs.
 *
 * This is the same layout as `pagh-riscv/userland/user.ld` (the kernel's
 * fixtures use it), kept in sync by hand because the two repositories are
 * independent:
 *
 *   - the image starts at 0x1_0000, inside the kernel's user window
 *     (0x10000 .. 0x3fff_0000), away from the kernel identity map at
 *     0x8000_0000 and the MMIO windows;
 *   - `.text.entry` is first, so `_start` is exactly at the base address and
 *     `ENTRY(_start)` matches the first byte of the first PT_LOAD;
 *   - .text + .rodata are R+X, .data + .bss are R+W. rust-lld turns the split
 *     into two PT_LOAD program headers, which is what the kernel's ELF loader
 *     expects (and what its reject matrix exercises);
 *   - no TLS, no `.init_array`, no dynamic sections: one process, one image.
 *
 * Programs must not rely on any section that is not listed here; the kernel
 * loader only maps PT_LOAD segments.
 */
ENTRY(_start)

SECTIONS
{
    . = 0x10000;

    .text : ALIGN(4096) {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    /* Read-only data gets its own page (and therefore its own R-only PT_LOAD,
     * exactly as `pagh-riscv/userland/user.ld` does). */
    . = ALIGN(4096);
    .rodata : ALIGN(16) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
        *(.sdata2 .sdata2.*)
    }

    /* The second (R+W) segment starts on its own page. */
    . = ALIGN(4096);
    .data : ALIGN(16) {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.*)
    }

    . = ALIGN(4096);
    .bss : ALIGN(16) {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.comment)
        *(.riscv.attributes)
        *(.note .note.*)
    }
}