/* M-BOOTLOADER's application SRAM starts at 0x01C30000, immediately
   above both download plug-ins. The BSP's normal RAM starts at 0x01C40000.
   See m-bootloader/docs/SPEC.md, "RAM app". */
MEMORY { clipram (rw) : ORIGIN = 0x01C30000, LENGTH = 64K }
SECTIONS {
  .wavegraph_clipboard (NOLOAD) : ALIGN(32) {
    KEEP(*(.wavegraph.clipboard))
  } > clipram
}
ASSERT(ORIGIN(clipram) + LENGTH(clipram) <= ORIGIN(ram), "clipboard overlaps BSP RAM")
ASSERT(SIZEOF(.wavegraph_clipboard) <= LENGTH(clipram), "clipboard exceeds reserved RAM")

/* In XIP firmware, the update-stage workspace is free after application entry.
   WaveGraph always resets to the loader. It never returns to a RAM stage.
   See m-bootloader docs/SPEC.md "RAM" and pi32v2-isa's 2026-10-24 retention
   probe. Keep the bootloader, both download plug-ins and the clipboard separate. */
MEMORY { echoram (rw) : ORIGIN = 0x01C0C000, LENGTH = 80K }
SECTIONS {
  .wavegraph_echo (NOLOAD) : ALIGN(32) {
    KEEP(*(.wavegraph.echo))
  } > echoram
}
ASSERT(ORIGIN(echoram) + LENGTH(echoram) <= 0x01C20000, "echo overlaps download plug-in")
ASSERT(SIZEOF(.wavegraph_echo) <= LENGTH(echoram), "echo exceeds reserved RAM")
