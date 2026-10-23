// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

#if defined(__x86_64__)
/* The saved syscall RIP must identify the instruction following SYSCALL. */
extern long seccomp_ip_probe(void);
extern char seccomp_ip_after;
_Static_assert(SYS_getppid == 110,
	       "the probe uses the native x86_64 syscall ABI");
__asm__(".text\n.code64\n\n"
	".balign 16\n"
	".global seccomp_ip_probe\n"
	".type seccomp_ip_probe, @function\n"
	"seccomp_ip_probe:\n"
	"mov $110, %eax\n"
	"syscall\n"
	".global seccomp_ip_after\n"
	"seccomp_ip_after:\n"
	"ret\n"
	".size seccomp_ip_probe, .-seccomp_ip_probe\n");

static void instruction_pointer(void)
{
	no_new_privs();
	uintptr_t ip = (uintptr_t)&seccomp_ip_after;
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 5),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 8),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)ip, 0, 3),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 12),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)(ip >> 32), 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EBUSY),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	CHECK(install(code, 8));
	/* The assembly wrapper returns the raw negative errno. */
	CHECK_WITH(seccomp_ip_probe(), _ret == -EBUSY);
}

static void syscall_number_width(void)
{
	long pid = CHECK(syscall(SYS_getpid));
	unsigned long high = 0x1234567800000000UL;
	/* Native x86 dispatch ignores the high 32 bits even without a filter. */
	CHECK_WITH(syscall(high | SYS_getpid), _ret == pid);
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	CHECK_WITH(syscall(high | SYS_getpid), _ret == pid);
	CHECK_WITH(syscall(high | SYS_getppid), _ret == -1 && errno == EBUSY);
	/* Bit 30 belongs to the syscall number (the x32 ABI), not its high half. */
	deny_syscall(0x40000000UL | SYS_getpid, SECCOMP_RET_ERRNO | E2BIG);
	CHECK_WITH(syscall(0x40000000UL | SYS_getpid),
		   _ret == -1 && errno == E2BIG);
}
#endif

static void raw_arguments(void)
{
	no_new_privs();
	/* Inspect all six argument slots, including bits above the ABI's int width. */
	for (int arg = 0; arg < 6; ++arg) {
		struct sock_filter code[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 0),
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 5),
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
				 offsetof(struct seccomp_data, args) + arg * 8),
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0x89abcdef, 0, 3),
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
				 offsetof(struct seccomp_data, args) + arg * 8 +
					 4),
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0x12345678, 0, 1),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EBUSY),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		CHECK(install(code, 8));
		uint64_t args[6] = { 0 };
		args[arg] = UINT64_C(0x1234567889abcdef);
		CHECK_WITH(syscall(SYS_getppid, args[0], args[1], args[2],
				   args[3], args[4], args[5]),
			   _ret == -1 && errno == EBUSY);
	}
	CHECK(syscall(SYS_getppid, 0, 0, 0, 0, 0, 0));
}

static void input_length_and_architecture(void)
{
	no_new_privs();
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 4),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, NATIVE_ARCH, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_LD | BPF_W | BPF_LEN, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 64, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_LDX | BPF_W | BPF_LEN, 0),
		BPF_STMT(BPF_MISC | BPF_TXA, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 64, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	CHECK(install(code, 11));
	CHECK(syscall(SYS_getppid));
}

FN_TEST(arguments)
{
	TEST_RES(run_child(raw_arguments, 0), _ret == 0);
	TEST_RES(run_child(input_length_and_architecture, 0), _ret == 0);
#if defined(__x86_64__)
	TEST_RES(run_child(instruction_pointer, 0), _ret == 0);
	TEST_RES(run_child(syscall_number_width, 0), _ret == 0);
#endif
}
END_TEST()
