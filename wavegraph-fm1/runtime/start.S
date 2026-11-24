/* The BSP linker places this prefix immediately before its ordinary _start.
   m-bootloader enters with interrupts disabled. No stack frame is live when
   the paint loop runs, including on a warm jump from the bootloader. */
.section .entry.cpu1_probe,"ax",@progbits
.global fm1_cpu1_probe_reset
fm1_cpu1_probe_reset:
    goto 1f
    rts
1:  cli
    ssp = __sstack_top
    sp = __stack_top
    call wavegraph_hold_cpu1
    r0 = __cpu1_mailbox
    r1 = __cpu1_mailbox_end
    r2 = 0
2:  if (r0 >= r1) goto 3f
    [r0+0] = r2
    r0 = r0 + 4
    goto 2b
3:  r0 = __sstack_bottom
    r1 = __stack_top
    r2 = 0xa55ac33c
4:  if (r0 >= r1) goto _start
    [r0+0] = r2
    r0 = r0 + 4
    goto 4b

/* Count untouched bytes from the low end of each descending CPU0 stack.
   Called from the low-priority display task; interrupts stay enabled.
   No Rust reference is constructed to live stack memory. */
.section .text.wavegraph_stack,"ax",@progbits
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
