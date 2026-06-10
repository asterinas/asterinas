// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <stdint.h>
#include <asm/prctl.h>
#include <cpuid.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define CPUID_FSGSBASE (1U << 0)

static unsigned long gs_slots[2];

static int cpu_has_fsgsbase(void)
{
	unsigned int eax;
	unsigned int ebx;
	unsigned int ecx;
	unsigned int edx;

	if (!__get_cpuid_count(7, 0, &eax, &ebx, &ecx, &edx)) {
		return 0;
	}

	return ebx & CPUID_FSGSBASE;
}

static int arch_get_gs(uintptr_t *gs_base)
{
	return syscall(SYS_arch_prctl, ARCH_GET_GS, gs_base);
}

static int arch_set_gs(uintptr_t gs_base)
{
	return syscall(SYS_arch_prctl, ARCH_SET_GS, gs_base);
}

static uintptr_t read_gsbase(void)
{
	uintptr_t gs_base;

	asm volatile("rdgsbase %0" : "=r"(gs_base) : : "memory");
	return gs_base;
}

static void write_gsbase(uintptr_t gs_base)
{
	asm volatile("wrgsbase %0" : : "r"(gs_base) : "memory");
}

FN_TEST(sync_gsbase_from_cpu)
{
	SKIP_TEST_IF(!cpu_has_fsgsbase());

	uintptr_t got_gs;

	uintptr_t syscall_gs = (uintptr_t)&gs_slots[0];
	TEST_SUCC(arch_set_gs(syscall_gs));
	usleep(100); // Trigger a syscall and context switch.
	TEST_RES(read_gsbase(), _ret == syscall_gs);
	TEST_RES(arch_get_gs(&got_gs), got_gs == syscall_gs);

	uintptr_t fsgsbase_gs = (uintptr_t)&gs_slots[1];
	write_gsbase(fsgsbase_gs);
	usleep(100); // Trigger a syscall and context switch.
	TEST_RES(read_gsbase(), _ret == fsgsbase_gs);
	TEST_RES(arch_get_gs(&got_gs), got_gs == fsgsbase_gs);
}
END_TEST()
