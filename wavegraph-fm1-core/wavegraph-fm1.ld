/* Keep the BSP memory map and startup symbols. Graph initialization runs on
   the user stack. Nested panel, display and audio work uses the supervisor stack. */
INCLUDE fm1-assets.ld
PROVIDE(__wavegraph_job_reserve = 0);
SECTIONS {
  /* The BSP already reserves its ROM exchange and 1 KiB bootstrap and
     supervisor stacks in .cpu1_scratch. The running DSP has a separate USP. */
  .wavegraph_cpu1 (NOLOAD) : ALIGN(32) {
    __cpu1_mailbox = .;
    KEEP(*(.wavegraph.job))
    __cpu1_payload_end = .;
    . = MAX(., __cpu1_mailbox + __wavegraph_job_reserve);
    . = ALIGN(32);
    __cpu1_mailbox_end = .;
    __cpu1_ustack_bottom = .;
    . += 12K;
    __cpu1_ustack_top = .;
  } > ram
}
__sstack_top = __stack_top - 16K;
__sstack_bottom = __sstack_top - 20K;
ASSERT(__wavegraph_job_reserve == 0 || __cpu1_payload_end <= __cpu1_mailbox + __wavegraph_job_reserve,
       "WaveGraph block exceeds its mailbox")
ASSERT(__fm1_statics_end <= __cpu1_mailbox, "CPU1 storage overlaps BSP statics")
ASSERT(__cpu1_ustack_top + 1K <= __sstack_bottom, "WaveGraph needs a 1 KiB CPU0 stack margin")
/* The BSP's USB window experiments retained SRAM here across software reset.
   This app reserves one record, outside both stacks and the ROM update mailbox. */
__wavegraph_fault_sram = 0x01c7c000;
ASSERT(__stack_top <= __wavegraph_fault_sram && __wavegraph_fault_sram + 64 <= 0x01c7fd80,
       "USB fault record overlaps application RAM or the loader mailbox")
