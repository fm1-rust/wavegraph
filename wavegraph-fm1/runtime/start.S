/* Runs once on CPU0 after the BSP reset prefix established both CPU0
   stacks and held CPU1. No live Rust objects or stack frames exist yet. */
.section .text.wavegraph_init,"ax",@progbits
.global __wrap__start
__wrap__start:
    r0 = __cpu1_mailbox
    r1 = __cpu1_mailbox_end
    r2 = 0
2:  if (r0 >= r1) goto 3f
    [r0+0] = r2
    r0 = r0 + 4
    goto 2b
3:  r0 = __cpu1_ustack_bottom
    r1 = __cpu1_ustack_top
    r2 = 0xa55ac33c
4:  if (r0 >= r1) goto 5f
    [r0+0] = r2
    r0 = r0 + 4
    goto 4b
5:  r0 = __sstack_bottom
    r1 = __stack_top
    r2 = 0xa55ac33c
6:  if (r0 >= r1) goto 7f
    [r0+0] = r2
    r0 = r0 + 4
    goto 6b
7:  goto __real__start

/* The BSP ROM trampoline has already checked cnum, USP, SP and SSP on its
   bootstrap stacks. Switch only the active user stack. No return or unwinding
   through the bootstrap frame; CPU1 retains its BSP supervisor stack. */
.section .cpu1.text.wavegraph_entry,"ax",@progbits
.global wavegraph_dsp_enter
wavegraph_dsp_enter:
    sp = __cpu1_ustack_top
    goto wavegraph_dsp_main

/* Each core measures its own stacks. No Rust reference to live stack memory.
   The four-byte stride and aligned boundaries are fixed by the linker. */
.section .text.wavegraph_stack,"ax",@progbits
.global wavegraph_cpu1_usp_free
wavegraph_cpu1_usp_free:
    r0 = __cpu1_ustack_bottom
    r1 = __cpu1_ustack_top
    goto 1f
.global wavegraph_usp_free
wavegraph_usp_free:
    r0 = __sstack_top
    r1 = __stack_top
    goto 1f
.global wavegraph_ssp_free
wavegraph_ssp_free:
    r0 = __sstack_bottom
    r1 = __sstack_top
1:  [--sp] = {r4}
    r2 = r0
    r3 = 0xa55ac33c
2:  if (r2 >= r1) goto 3f
    r4 = [r2+0]
    if (r4 != r3) goto 3f
    r2 = r2 + 4
    goto 2b
3:  r0 = r2 - r0
    {r4} = [sp++]
    rts
