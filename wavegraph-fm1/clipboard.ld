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
