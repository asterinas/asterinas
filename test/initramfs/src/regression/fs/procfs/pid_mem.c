// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <stdint.h>
#include <string.h>
#include <linux/capability.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

#define PAGE_SIZE 4096
#define ORIG_STR "ORIGINAL"
#define NEW_STR "MODIFIED"
#define FILE_NAME "testfile"

static int access_from_sibling(int drop_cap, int switch_user);
static int access_from_parent(int drop_cap);

FN_TEST(proc_mem_alien)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	int fd = TEST_SUCC(open(FILE_NAME, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, ORIG_STR, strlen(ORIG_STR) + 1));
	TEST_SUCC(close(fd));

	int pipe_c2p[2], pipe_p2c[2];
	TEST_SUCC(pipe(pipe_c2p));
	TEST_SUCC(pipe(pipe_p2c));

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		// ===== Child =====
		CHECK(close(pipe_c2p[0]));
		CHECK(close(pipe_p2c[1]));

		int fd = CHECK(open(FILE_NAME, O_RDONLY));
		// The parent should successfully read from and (force) write to this
		// memory region via `/proc/pid/mem`, although it isn't `PROT_WRITE`.
		void *addr = CHECK_WITH(mmap(NULL, PAGE_SIZE, PROT_READ,
					     MAP_PRIVATE, fd, 0),
					_ret != MAP_FAILED);
		CHECK(write(pipe_c2p[1], &addr, sizeof(addr)));

		// Wait for the parent to read and write.
		char ack;
		CHECK(read(pipe_p2c[0], &ack, 1));

		// Check that the memory was modified by the parent.
		CHECK_WITH(memcmp(addr, NEW_STR, strlen(NEW_STR)), _ret == 0);

		// Check that the file was not modified.
		char filebuf[64] = { 0 };
		CHECK(lseek(fd, 0, SEEK_SET));
		CHECK(read(fd, filebuf, sizeof(filebuf)));
		CHECK_WITH(strncmp(filebuf, ORIG_STR, strlen(ORIG_STR)),
			   _ret == 0);

		CHECK(munmap(addr, PAGE_SIZE));
		CHECK(close(fd));
		CHECK(close(pipe_c2p[1]));
		CHECK(close(pipe_p2c[0]));
		exit(EXIT_SUCCESS);
	}

	// ===== Parent =====
	TEST_SUCC(close(pipe_c2p[1]));
	TEST_SUCC(close(pipe_p2c[0]));

	void *child_vaddr;
	TEST_SUCC(read(pipe_c2p[0], &child_vaddr, sizeof(child_vaddr)));

	char mempath[256];
	snprintf(mempath, sizeof(mempath), "/proc/%d/mem", (int)child);
	int proc_mem_fd = TEST_SUCC(open(mempath, O_RDWR));

	// Read from the child's memory via /proc/pid/mem.
	// This will trigger a read page fault on the child process.
	TEST_SUCC(lseek(proc_mem_fd, (off_t)child_vaddr, SEEK_SET));
	char readbuf[64] = { 0 };
	TEST_SUCC(read(proc_mem_fd, readbuf, sizeof(readbuf)));
	TEST_RES(strncmp(readbuf, ORIG_STR, strlen(ORIG_STR)), _ret == 0);

	// Write to the child's memory via /proc/pid/mem.
	// This will trigger a write page fault and perform COW on the child process.
	TEST_SUCC(lseek(proc_mem_fd, (off_t)child_vaddr, SEEK_SET));
	TEST_SUCC(write(proc_mem_fd, NEW_STR, strlen(NEW_STR) + 1));
	TEST_SUCC(close(proc_mem_fd));

	TEST_SUCC(write(pipe_p2c[1], "X", 1));

	int status;
	TEST_RES(wait4(child, &status, 0, NULL),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	TEST_SUCC(close(pipe_c2p[0]));
	TEST_SUCC(close(pipe_p2c[1]));
	TEST_SUCC(unlink(FILE_NAME));
}
END_TEST()

FN_TEST(proc_mem_local)
{
	int fd = TEST_SUCC(open(FILE_NAME, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));
	TEST_SUCC(write(fd, ORIG_STR, strlen(ORIG_STR) + 1));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(FILE_NAME, O_RDONLY));
	void *addr1 =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_PRIVATE, fd, 0));
	void *addr2 = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));

	int proc_mem_fd = TEST_SUCC(open("/proc/self/mem", O_RDWR));
	TEST_SUCC(lseek(proc_mem_fd, (off_t)addr1, SEEK_SET));
	// This `read` will first trigger a page fault on `addr1` to load the
	// corresponding file page into memory. Then it will trigger a write
	// page fault on `addr2` to copy the content of that file page.
	TEST_SUCC(read(proc_mem_fd, addr2, PAGE_SIZE));
	TEST_RES(strncmp(addr2, ORIG_STR, strlen(ORIG_STR)), _ret == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(close(proc_mem_fd));
	TEST_SUCC(munmap(addr1, PAGE_SIZE));
	TEST_SUCC(munmap(addr2, PAGE_SIZE));
	TEST_SUCC(unlink(FILE_NAME));
}
END_TEST()

FN_TEST(proc_mem_yama_scope)
{
	int old_scope = read_yama_scope();

	// `YAMA_SCOPE_NO_ATTACH` is immutable once set.
	SKIP_TEST_IF(old_scope == YAMA_SCOPE_NO_ATTACH);

	// 0. From another user without `CAP_SYS_PTRACE`: denied at Yama scopes 0, 1 and 2.
	write_yama_scope(0);
	TEST_RES(access_from_sibling(1, 1), _ret == EACCES);
	write_yama_scope(1);
	TEST_RES(access_from_sibling(1, 1), _ret == EACCES);
	write_yama_scope(2);
	TEST_RES(access_from_sibling(1, 1), _ret == EACCES);

	// 1. From a sibling without `CAP_SYS_PTRACE`: allowed at 0, denied at 1 and 2.
	write_yama_scope(0);
	TEST_RES(access_from_sibling(1, 0), _ret == 0);
	write_yama_scope(1);
	TEST_RES(access_from_sibling(1, 0), _ret == EACCES);
	write_yama_scope(2);
	TEST_RES(access_from_sibling(1, 0), _ret == EACCES);

	// 2. From the parent without `CAP_SYS_PTRACE`: allowed at 0 and 1, denied at 2.
	write_yama_scope(0);
	TEST_RES(access_from_parent(1), _ret == 0);
	write_yama_scope(1);
	TEST_RES(access_from_parent(1), _ret == 0);
	write_yama_scope(2);
	TEST_RES(access_from_parent(1), _ret == EACCES);

	// 3. From the parent with `CAP_SYS_PTRACE`: allowed at 0, 1 and 2.
	write_yama_scope(0);
	TEST_RES(access_from_parent(0), _ret == 0);
	write_yama_scope(1);
	TEST_RES(access_from_parent(0), _ret == 0);
	write_yama_scope(2);
	TEST_RES(access_from_parent(0), _ret == 0);

	write_yama_scope(old_scope);
}
END_TEST()

static void drop_cap_sys_ptrace(void)
{
	struct __user_cap_header_struct hdr = {
		.version = _LINUX_CAPABILITY_VERSION_3,
	};
	struct __user_cap_data_struct capdat[2] = { 0 };

	CHECK(syscall(SYS_capget, &hdr, &capdat));

	capdat[0].effective &= ~(1 << CAP_SYS_PTRACE);
	capdat[0].permitted &= ~(1 << CAP_SYS_PTRACE);
	capdat[0].inheritable &= ~(1 << CAP_SYS_PTRACE);

	CHECK(syscall(SYS_capset, &hdr, &capdat));
}

static void drop_to_another_user(void)
{
	CHECK(setresgid(65534, 65534, 65534));
	CHECK(setresuid(65534, 65534, 65534));
}

// Tries to open `/proc/[target]/mem` for read and write.
//
// Returns 0 on success, or the value of the errno on failure.
static int try_open_proc_mem(pid_t target)
{
	char mempath[256];
	CHECK(snprintf(mempath, sizeof(mempath), "/proc/%d/mem", (int)target));

	errno = 0;
	int fd = open(mempath, O_RDWR);
	if (fd >= 0) {
		CHECK(close(fd));
		return 0;
	}
	return errno;
}

static int access_from_sibling(int drop_cap, int switch_user)
{
	int pipe_block[2];
	CHECK(pipe(pipe_block));

	pid_t target = CHECK(fork());
	if (target == 0) {
		CHECK(close(pipe_block[1]));
		if (drop_cap) {
			// Keep the target capability same with the accessor,
			// to pass Linux commoncap checks.
			drop_cap_sys_ptrace();
		}
		char ch;
		CHECK(read(pipe_block[0], &ch, 1));
		CHECK(close(pipe_block[0]));
		exit(EXIT_SUCCESS);
	}
	CHECK(close(pipe_block[0]));

	int pipe_res[2];
	CHECK(pipe(pipe_res));
	pid_t accessor = CHECK(fork());
	if (accessor == 0) {
		CHECK(close(pipe_res[0]));
		if (drop_cap) {
			drop_cap_sys_ptrace();
		}
		if (switch_user) {
			drop_to_another_user();
		}

		int res = try_open_proc_mem(target);
		CHECK(write(pipe_res[1], &res, sizeof(res)));
		CHECK(close(pipe_res[1]));
		exit(EXIT_SUCCESS);
	}
	CHECK(close(pipe_res[1]));

	int res = 0, status = 0;
	CHECK(read(pipe_res[0], &res, sizeof(res)));
	CHECK(close(pipe_res[0]));
	CHECK_WITH(wait4(accessor, &status, 0, NULL),
		   _ret == accessor && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);

	CHECK(write(pipe_block[1], "A", 1));
	CHECK(close(pipe_block[1]));
	CHECK_WITH(wait4(target, &status, 0, NULL),
		   _ret == target && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);
	return res;
}

static int access_from_parent(int drop_cap)
{
	int pipe_res[2];
	CHECK(pipe(pipe_res));

	pid_t parent = CHECK(fork());
	if (parent == 0) {
		CHECK(close(pipe_res[0]));
		if (drop_cap) {
			drop_cap_sys_ptrace();
		}

		int pipe_block[2];
		CHECK(pipe(pipe_block));
		pid_t target = CHECK(fork());
		if (target == 0) {
			CHECK(close(pipe_block[1]));
			char ch;
			CHECK(read(pipe_block[0], &ch, 1));
			CHECK(close(pipe_block[0]));
			exit(EXIT_SUCCESS);
		}
		CHECK(close(pipe_block[0]));

		int res = try_open_proc_mem(target);

		CHECK(write(pipe_block[1], "A", 1));
		CHECK(close(pipe_block[1]));
		int status = 0;
		CHECK_WITH(wait4(target, &status, 0, NULL),
			   _ret == target && WIFEXITED(status) &&
				   WEXITSTATUS(status) == EXIT_SUCCESS);

		CHECK(write(pipe_res[1], &res, sizeof(res)));
		CHECK(close(pipe_res[1]));
		exit(EXIT_SUCCESS);
	}
	CHECK(close(pipe_res[1]));

	int res = 0, status = 0;
	CHECK(read(pipe_res[0], &res, sizeof(res)));
	CHECK(close(pipe_res[0]));
	CHECK_WITH(wait4(parent, &status, 0, NULL),
		   _ret == parent && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);
	return res;
}
