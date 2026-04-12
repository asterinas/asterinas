// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/wait.h>

#include "../../common/test.h"

#define TEST_CHILD_RECEIVE_FAULT_SIGNAL(func, sig)                        \
	pid = TEST_SUCC(fork());                                          \
	if (pid == 0) {                                                   \
		func();                                                   \
		exit(EXIT_FAILURE);                                       \
	}                                                                 \
                                                                          \
	TEST_RES(waitpid(pid, &status, 0), _ret == pid &&                 \
						   WIFSIGNALED(status) && \
						   WTERMSIG(status) == (sig));

// This generates #OF. The kernel should respond with a SIGSEGV.
static void generate_of(void)
{
	asm volatile("int $4");
}

FN_TEST(of_exception_to_sigsegv)
{
	pid_t pid;
	int status;

	TEST_CHILD_RECEIVE_FAULT_SIGNAL(generate_of, SIGSEGV);
}
END_TEST()

// This generates #SS. The kernel should respond with a SIGBUS.
static void generate_ss(void)
{
	asm volatile("movabs $0x8000000000000000, %%rax\n\t"
		     "mov %%rax, %%rsp\n\t"
		     "push %%rax\n\t"
		     :
		     :
		     : "rax", "memory");
}

FN_TEST(ss_exception_to_sigsegv)
{
	pid_t pid;
	int status;

	TEST_CHILD_RECEIVE_FAULT_SIGNAL(generate_ss, SIGBUS);
}
END_TEST()

static void try_generate_br_via_bound(void)
{
	// bound %eax, (%eax)
	asm volatile(".byte 0x62, 0x00");
}

static void try_generate_br_via_int5(void)
{
	asm volatile("int $5");
}

static void try_generate_br_via_mpx(void)
{
	// bndcl (%rax), %bnd0
	asm volatile(".byte 0xf3, 0x0f, 0x1a, 0x00");
}

FN_TEST(br_exception_cannot_be_generated)
{
	pid_t pid;
	int status;

	// The `bound` instruction is not a valid instruction in 64-bit mode.
	// So it generates SIGILL.
	TEST_CHILD_RECEIVE_FAULT_SIGNAL(try_generate_br_via_bound, SIGILL);

	// The `int` instruction cannot trigger #BR: the vector-5 gate has DPL 0,
	// so it is callable only from Ring 0.
	TEST_CHILD_RECEIVE_FAULT_SIGNAL(try_generate_br_via_int5, SIGSEGV);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		try_generate_br_via_mpx();
		exit(EXIT_SUCCESS);
	}

	// "When Intel MPX is not enabled or not present, all Intel MPX
	// instructions behave as NOP."
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
