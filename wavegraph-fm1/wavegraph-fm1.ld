/* Keep the BSP memory map and startup symbols. Graph initialization runs on
   the user stack. Nested panel, display and audio work uses the supervisor stack. */
INCLUDE fm1.ld
__sstack_top = __stack_top - 16K;
ASSERT(__bss_end <= __sstack_top - 24K, "WaveGraph statics overlap the interrupt stack")
