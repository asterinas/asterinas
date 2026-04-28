// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

static int expected_si_code;
static void *expected_si_addr;

static void sigsegv_handler(int sig, siginfo_t *info, void *context)
{
	(void)context;

	if (sig != SIGSEGV) {
		_exit(100);
	}
	if (info->si_code != expected_si_code) {
		_exit(101);
	}
	if (expected_si_addr != NULL && info->si_addr != expected_si_addr) {
		_exit(102);
	}

	_exit(0);
}

static void install_sigsegv_handler(void)
{
	struct sigaction action = { 0 };
	action.sa_sigaction = sigsegv_handler;
	action.sa_flags = SA_SIGINFO;
	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGSEGV, &action, NULL));
}

static void trigger_unmapped_fault(void)
{
	volatile char value = *(volatile char *)0;
	(void)value;
}

static void *prot_none_addr;

static void trigger_prot_none_fault(void)
{
	volatile char value = *(volatile char *)prot_none_addr;
	(void)value;
}

static int run_sigsegv_case(void (*trigger)(void), int si_code,
			    void *fault_addr)
{
	pid_t pid = fork();
	if (pid < 0) {
		return -1;
	}
	if (pid == 0) {
		expected_si_code = si_code;
		expected_si_addr = fault_addr;
		install_sigsegv_handler();
		trigger();
		_exit(103);
	}

	int status;
	if (waitpid(pid, &status, 0) != pid) {
		return -1;
	}
	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		errno = EINVAL;
		return -1;
	}
	return 0;
}

FN_TEST(sigsegv_si_code)
{
	TEST_SUCC(run_sigsegv_case(trigger_unmapped_fault, SEGV_MAPERR, NULL));

	prot_none_addr = CHECK_WITH(mmap(NULL, PAGE_SIZE, PROT_NONE,
					 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
				    _ret != MAP_FAILED);
	TEST_SUCC(run_sigsegv_case(trigger_prot_none_fault, SEGV_ACCERR,
				   prot_none_addr));
	TEST_RES(munmap(prot_none_addr, PAGE_SIZE), _ret == 0);
}
END_TEST()
