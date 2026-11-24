/* Keep the BSP memory map and startup symbols. Graph initialization runs on
   the user stack. Nested panel, display and audio work uses the supervisor stack. */
/* Keep the native BSP/RTICX shims and memcpy in direct-call range of the SRAM panel ISR. */
SECTIONS { .wavegraph_near : { *(.text .text.memcpy) } } INSERT BEFORE .text;
INCLUDE fm1.ld
/* Physical flash 0x93000..0xE0000, before Felucca's update staging area.
   XIP logical offset zero corresponds to physical 0x4000. */
MEMORY {
  assets (r) : ORIGIN = ORIGIN(flash) + LENGTH(flash), LENGTH = 0x4D000
}
SECTIONS {
  /* Separate 32-byte publication lines, owned job data and mono Q23 output.
     The reset prefix zeroes the whole mailbox before Rust or interrupts run. */
  .wavegraph_cpu1 (NOLOAD) : ALIGN(32) {
    __cpu1_mailbox = .;
    __cpu1_request = .;
    . += 32;
    __cpu1_response = .;
    . += 32;
    KEEP(*(.wavegraph.job))
    __cpu1_payload_end = .;
    . = MAX(., __cpu1_mailbox + 4672);
    . = ALIGN(32);
    __cpu1_mailbox_end = .;
    __cpu1_ustack_bottom = .;
    . += 8K;
    __cpu1_ustack_top = .;
    __cpu1_sstack_bottom = .;
    . += 1K;
    __cpu1_sstack_top = .;
  } > ram
  .wavegraph_assets : ALIGN(4) {
    *(.wavegraph_assets)
    KEEP(*(.wavegraph_assets.notices))
  } > assets
}
__sstack_top = __stack_top - 16K;
__sstack_bottom = __sstack_top - 26K;
ASSERT(__cpu1_payload_end <= __cpu1_mailbox + 4672, "WaveGraph block exceeds its mailbox")
ASSERT(__fm1_statics_end <= __cpu1_mailbox, "CPU1 storage overlaps BSP statics")
ASSERT(__cpu1_sstack_top + 1K <= __sstack_bottom, "WaveGraph needs a 1 KiB CPU0 stack margin")
