/* Keep the BSP memory map and startup symbols. Graph initialization runs on
   the user stack. Nested panel, display and audio work uses the supervisor stack. */
INCLUDE fm1.ld
/* Physical flash 0x93000..0xE0000, before Felucca's update staging area.
   XIP logical offset zero corresponds to physical 0x4000. */
MEMORY {
  assets (r) : ORIGIN = ORIGIN(flash) + LENGTH(flash), LENGTH = 0x4D000
}
SECTIONS {
  .wavegraph_assets : ALIGN(4) { *(.wavegraph_assets) } > assets
}
__sstack_top = __stack_top - 16K;
ASSERT(__bss_end <= __sstack_top - 24K, "WaveGraph statics overlap the interrupt stack")
