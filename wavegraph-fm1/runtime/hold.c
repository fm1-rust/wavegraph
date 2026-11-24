/* Hold-only startup from fm1/runtime/cpu1_probe.c at fm1/v0.2.0-rc.1.
   The BSP packages its hold together with the probe's release and ROM entry.
   This image uses only the pre-CRT hold sequence. */
#include <stdint.h>
#define REG(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define C1_CON     REG(0x01eee004)
#define DBG_WR_EN  REG(0x01eee240)
#define DBG_MSG_CLR REG(0x01eee248)
#define DBG_EN     REG(0x01eee340)
#define HOLD (1u << 1)
#define RUN  (1u << 3)

static inline void sync(void) { __asm__ volatile("csync" ::: "memory"); }
static void debug_access(uint32_t enabled) {
    if ((DBG_WR_EN & 1u) != enabled)
        DBG_WR_EN = 0xe7;
    sync();
}

void wavegraph_hold_cpu1(void) {
    const uint32_t bus_causes = 0x003f0030;
    uint32_t access = DBG_WR_EN & 1u;
    debug_access(1);
    uint32_t debug = DBG_EN;
    DBG_EN = debug & ~bus_causes;
    sync();
    C1_CON |= HOLD;
    C1_CON &= ~RUN;
    sync();
    for (unsigned n = 0; n < 256; ++n)
        __asm__ volatile("nop");
    DBG_MSG_CLR = bus_causes;
    sync();
    DBG_EN = debug;
    sync();
    debug_access(access);
    if ((C1_CON & (HOLD | RUN)) != HOLD)
        for (;;)
            __asm__ volatile("nop");
}
