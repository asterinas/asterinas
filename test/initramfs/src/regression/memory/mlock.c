// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <stdint.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../common/capability.h"
#include "../common/test.h"

#define PAGE_SIZE 4096

static void *map_pages(size_t nr_pages)
{
	void *addr = mmap(NULL, nr_pages * PAGE_SIZE, PROT_READ | PROT_WRITE,
			  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

	if (addr == MAP_FAILED) {
		return NULL;
	}

	return addr;
}

static int set_memlock_limit(rlim_t limit)
{
	struct rlimit rlim = {
		.rlim_cur = limit,
		.rlim_max = limit,
	};

	return setrlimit(RLIMIT_MEMLOCK, &rlim);
}

static int run_quota_test(void (*test_func)(void))
{
	pid_t pid = CHECK(fork());
	int status;

	if (pid == 0) {
		drop_capability(CAP_IPC_LOCK);
		test_func();
		_exit(0);
	}

	CHECK(waitpid(pid, &status, 0));
	if (!WIFEXITED(status)) {
		return -1;
	}

	return WEXITSTATUS(status);
}

static void exit_if_failed(int condition)
{
	if (!condition) {
		_exit(1);
	}
}

FN_TEST(mlock_and_munlock)
{
	void *addr = map_pages(1);

	TEST_RES((long)addr, _ret != 0);
	TEST_SUCC(mlock(addr, PAGE_SIZE));
	TEST_SUCC(munlock(addr, PAGE_SIZE));
	TEST_SUCC(munmap(addr, PAGE_SIZE));
}
END_TEST()

FN_TEST(mlock_error_cases)
{
	void *addr = map_pages(2);

	TEST_RES((long)addr, _ret != 0);
	TEST_SUCC(munmap(addr + PAGE_SIZE, PAGE_SIZE));
	TEST_SUCC(mlock(addr + 1, PAGE_SIZE - 1));
	TEST_SUCC(munlock(addr + 1, PAGE_SIZE - 1));
	TEST_ERRNO(mlock(addr + 1, PAGE_SIZE), ENOMEM);
	TEST_ERRNO(mlock(addr, PAGE_SIZE * 2), ENOMEM);
	TEST_ERRNO(munlock(addr, PAGE_SIZE * 2), ENOMEM);
	TEST_SUCC(munmap(addr, PAGE_SIZE));
}
END_TEST()

static void quota_mlock_does_not_charge_twice(void)
{
	void *addr = map_pages(1);

	exit_if_failed(addr != NULL);
	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlock(addr, PAGE_SIZE) == 0);
	exit_if_failed(mlock(addr, PAGE_SIZE) == 0);
	errno = 0;
	exit_if_failed(mlock(addr, PAGE_SIZE * 2) == -1 && errno == ENOMEM);
	exit_if_failed(munlock(addr, PAGE_SIZE) == 0);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
}

FN_TEST(mlock_does_not_charge_twice)
{
	TEST_RES(run_quota_test(quota_mlock_does_not_charge_twice), _ret == 0);
}
END_TEST()

static void quota_mlockall_current(void)
{
	void *addr = map_pages(1);

	exit_if_failed(addr != NULL);
	exit_if_failed(set_memlock_limit(RLIM_INFINITY) == 0);
	exit_if_failed(mlockall(MCL_CURRENT) == 0);
	exit_if_failed(munlockall() == 0);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
}

FN_TEST(mlockall_current)
{
	TEST_RES(run_quota_test(quota_mlockall_current), _ret == 0);
}
END_TEST()

static void quota_mlockall_future(void)
{
	void *addr;

	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlockall(MCL_FUTURE) == 0);
	addr = mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
		    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	exit_if_failed(addr != MAP_FAILED);
	errno = 0;
	exit_if_failed(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) == MAP_FAILED &&
		       errno == ENOMEM);
	exit_if_failed(munlockall() == 0);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);

	addr = mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
		    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	exit_if_failed(addr != MAP_FAILED);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
}

FN_TEST(mlockall_future)
{
	TEST_RES(run_quota_test(quota_mlockall_future), _ret == 0);
}
END_TEST()

static void quota_mmap_fixed_future_preserves_target(void)
{
	char *new_addr = map_pages(1);
	void *mapped;

	exit_if_failed(new_addr != NULL);
	new_addr[0] = 0x5a;
	exit_if_failed(mlockall(MCL_FUTURE) == 0);
	errno = 0;
	mapped = mmap(new_addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	exit_if_failed(mapped == MAP_FAILED);
	exit_if_failed(errno == ENOMEM);
	exit_if_failed(new_addr[0] == 0x5a);
	exit_if_failed(munmap(new_addr, PAGE_SIZE) == 0);
	exit_if_failed(munlockall() == 0);
}

FN_TEST(mmap_fixed_future_preserves_target)
{
	TEST_RES(run_quota_test(quota_mmap_fixed_future_preserves_target),
		 _ret == 0);
}
END_TEST()

static void quota_mremap_locked_resize(void)
{
	void *addr = map_pages(2);
	void *blocker;
	void *remapped;

	exit_if_failed(addr != NULL);
	blocker = (char *)addr + PAGE_SIZE;
	exit_if_failed(munmap(blocker, PAGE_SIZE) == 0);
	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlock(addr, PAGE_SIZE) == 0);
	errno = 0;
	remapped = mremap(addr, PAGE_SIZE, PAGE_SIZE * 2, 0);
	exit_if_failed(remapped == MAP_FAILED);
	exit_if_failed(errno == ENOMEM);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
}

FN_TEST(mremap_locked_resize_checks_quota)
{
	TEST_RES(run_quota_test(quota_mremap_locked_resize), _ret == 0);
}
END_TEST()

static void quota_mremap_locked_move_preserves_lock(void)
{
	void *addr = map_pages(1);
	void *new_addr = map_pages(1);

	exit_if_failed(addr != NULL);
	exit_if_failed(new_addr != NULL);
	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlock(addr, PAGE_SIZE) == 0);
	new_addr = mremap(addr, PAGE_SIZE, PAGE_SIZE,
			  MREMAP_MAYMOVE | MREMAP_FIXED, new_addr);
	exit_if_failed(new_addr != MAP_FAILED);
	exit_if_failed(mlock(new_addr, PAGE_SIZE) == 0);
	exit_if_failed(munmap(new_addr, PAGE_SIZE) == 0);
}

FN_TEST(mremap_locked_move_preserves_lock)
{
	TEST_RES(run_quota_test(quota_mremap_locked_move_preserves_lock),
		 _ret == 0);
}
END_TEST()

static void quota_mremap_locked_maymove_checks_quota(void)
{
	void *addr = map_pages(2);
	void *blocker;
	void *remapped;

	exit_if_failed(addr != NULL);
	blocker = (char *)addr + PAGE_SIZE;
	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlock(addr, PAGE_SIZE) == 0);
	errno = 0;
	remapped = mremap(addr, PAGE_SIZE, PAGE_SIZE * 2, MREMAP_MAYMOVE);
	exit_if_failed(remapped == MAP_FAILED);
	exit_if_failed(errno == ENOMEM);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
	exit_if_failed(munmap(blocker, PAGE_SIZE) == 0);
}

FN_TEST(mremap_locked_maymove_checks_quota)
{
	TEST_RES(run_quota_test(quota_mremap_locked_maymove_checks_quota),
		 _ret == 0);
}
END_TEST()

static void quota_mremap_future_preserves_target(void)
{
	char *addr = map_pages(1);
	char *new_addr = map_pages(2);

	exit_if_failed(addr != NULL);
	exit_if_failed(new_addr != NULL);
	new_addr[0] = 0x5a;
	exit_if_failed(mlockall(MCL_FUTURE) == 0);
	errno = 0;
	exit_if_failed(mremap(addr, PAGE_SIZE, PAGE_SIZE * 2,
			      MREMAP_MAYMOVE | MREMAP_FIXED,
			      new_addr) == MAP_FAILED);
	exit_if_failed(errno == ENOMEM);
	exit_if_failed(new_addr[0] == 0x5a);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
	exit_if_failed(munmap(new_addr, PAGE_SIZE * 2) == 0);
	exit_if_failed(munlockall() == 0);
}

FN_TEST(mremap_fixed_future_expansion_preserves_target)
{
	TEST_RES(run_quota_test(quota_mremap_future_preserves_target),
		 _ret == 0);
}
END_TEST()

static void quota_mmap_fixed_future_empty_target_succeeds(void)
{
	char *addr = map_pages(1);
	void *mapped;

	exit_if_failed(addr != NULL);
	exit_if_failed(munmap(addr, PAGE_SIZE) == 0);
	exit_if_failed(mlockall(MCL_FUTURE) == 0);
	mapped = mmap(addr, PAGE_SIZE, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	exit_if_failed(mapped == addr);
	exit_if_failed(munmap(mapped, PAGE_SIZE) == 0);
	exit_if_failed(munlockall() == 0);
}

FN_TEST(mmap_fixed_future_empty_target_succeeds)
{
	TEST_RES(run_quota_test(quota_mmap_fixed_future_empty_target_succeeds),
		 _ret == 0);
}
END_TEST()

static void quota_mremap_future_empty_target_succeeds(void)
{
	char *addr = map_pages(1);
	char *new_addr = map_pages(2);
	void *remapped;

	exit_if_failed(addr != NULL);
	exit_if_failed(new_addr != NULL);
	exit_if_failed(munmap(new_addr, PAGE_SIZE * 2) == 0);
	exit_if_failed(mlockall(MCL_FUTURE) == 0);
	remapped = mremap(addr, PAGE_SIZE, PAGE_SIZE * 2,
			  MREMAP_MAYMOVE | MREMAP_FIXED, new_addr);
	exit_if_failed(remapped == new_addr);
	exit_if_failed(munmap(remapped, PAGE_SIZE * 2) == 0);
	exit_if_failed(munlockall() == 0);
}

FN_TEST(mremap_fixed_future_empty_target_succeeds)
{
	TEST_RES(run_quota_test(quota_mremap_future_empty_target_succeeds),
		 _ret == 0);
}
END_TEST()

static void quota_brk_locked_growth_checks_quota(void)
{
	uintptr_t current_break = (uintptr_t)sbrk(0);
	void *heap_page = (void *)((current_break - 1) & ~(PAGE_SIZE - 1));
	uintptr_t new_break = current_break + PAGE_SIZE;
	long actual_break;

	exit_if_failed(current_break != (uintptr_t)-1);
	exit_if_failed(set_memlock_limit(PAGE_SIZE) == 0);
	exit_if_failed(mlock(heap_page, PAGE_SIZE) == 0);
	actual_break = syscall(SYS_brk, new_break);
	exit_if_failed(actual_break == (long)current_break);
	exit_if_failed(munlock(heap_page, PAGE_SIZE) == 0);
}

FN_TEST(brk_locked_growth_checks_quota)
{
	TEST_RES(run_quota_test(quota_brk_locked_growth_checks_quota),
		 _ret == 0);
}
END_TEST()

FN_TEST(mlockall_invalid_flags)
{
	TEST_ERRNO(mlockall(0), EINVAL);
	TEST_ERRNO(mlockall(MCL_ONFAULT), EINVAL);
	TEST_ERRNO(mlockall(MCL_CURRENT | MCL_ONFAULT), EINVAL);
}
END_TEST()
