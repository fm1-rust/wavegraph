/* Keep the BSP memory map and startup symbols. Graph initialization runs on
   the user stack. Nested panel, display and audio work uses the supervisor stack. */
/* The panel ISR, 303 renderer and CPU1 worker run in SRAM. Their direct calls
   reach only the lower part of XIP as firmware grows. */
SECTIONS {
  .wavegraph_near : { *(.text .text.memcpy .text.wavegraph_boot .text.wavegraph_stack .text.*decay_exp* .text.*rticx_pi32v2*Restore*Drop4drop
      .text.*fm1*panel*select .text.*TouchControls*start_column
      .text.*option*expect_failed .text.*panicking*panic_bounds_check) }
} INSERT BEFORE .text;
INCLUDE fm1.ld
/* Physical flash 0x93000..0xE0000, before Felucca's update staging area.
   XIP logical offset zero corresponds to physical 0x4000. */
MEMORY {
  assets (r) : ORIGIN = ORIGIN(flash) + LENGTH(flash), LENGTH = 0x4D000
}
SECTIONS {
  /* The BSP already reserves its ROM exchange and 1 KiB bootstrap and
     supervisor stacks in .cpu1_scratch. The running DSP has a separate USP. */
  .wavegraph_cpu1 (NOLOAD) : ALIGN(32) {
    __cpu1_mailbox = .;
    KEEP(*(.wavegraph.job))
    __cpu1_payload_end = .;
    . = MAX(., __cpu1_mailbox + 4864);
    . = ALIGN(32);
    __cpu1_mailbox_end = .;
    __cpu1_ustack_bottom = .;
    . += 12K;
    __cpu1_ustack_top = .;
  } > ram
  .wavegraph_assets : ALIGN(4) {
    *(.wavegraph_assets)
    KEEP(*(.wavegraph_assets.notices))
  } > assets
}
__sstack_top = __stack_top - 16K;
__sstack_bottom = __sstack_top - 20K;
ASSERT(__cpu1_payload_end <= __cpu1_mailbox + 4864, "WaveGraph block exceeds its mailbox")
ASSERT(__fm1_statics_end <= __cpu1_mailbox, "CPU1 storage overlaps BSP statics")
ASSERT(__cpu1_ustack_top + 1K <= __sstack_bottom, "WaveGraph needs a 1 KiB CPU0 stack margin")
