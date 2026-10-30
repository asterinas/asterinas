// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define PAGE_SIZE 4096

#define NX_DIR "/tmp/mf_noexec"
#define ND_DIR "/tmp/mf_nodev"
#define NSF_DIR "/tmp/mf_nosymfollow"

static void copy_file(const char *src, const char *dst)
{
	int src_fd = CHECK(open(src, O_RDONLY));
	int dst_fd = CHECK(open(dst, O_WRONLY | O_CREAT | O_TRUNC, 0755));

	char buf[4096];
	ssize_t n;
	while ((n = read(src_fd, buf, sizeof(buf))) > 0) {
		CHECK_WITH(write(dst_fd, buf, (size_t)n), _ret == n);
	}

	CHECK(close(src_fd));
	CHECK(close(dst_fd));
}

// Checks the child's exec result against the expected errno.
static void exec_and_expect(const char *path, int want_errno)
{
	pid_t pid = CHECK(fork());

	if (pid == 0) {
		execl(path, "sh", "-c", "exit 0", (char *)NULL);
		_exit(errno);
	}

	int status = 0;
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFEXITED(status) &&
			   WEXITSTATUS(status) == want_errno);
}

FN_SETUP(mount_fixtures)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));
	CHECK_WITH(mkdir(NX_DIR, 0755), _ret == 0 || errno == EEXIST);
	CHECK_WITH(mkdir(ND_DIR, 0755), _ret == 0 || errno == EEXIST);
	CHECK_WITH(mkdir(NSF_DIR, 0755), _ret == 0 || errno == EEXIST);
	CHECK_WITH(unlink(NX_DIR "/sh"), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(ND_DIR "/null"), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(NSF_DIR "/link"), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink("/tmp/mf_link_plain"), _ret == 0 || errno == ENOENT);
	CHECK(mount("tmpfs", NX_DIR, "tmpfs", MS_NOEXEC, NULL));
	CHECK(mount("tmpfs", ND_DIR, "tmpfs", MS_NODEV, NULL));
	CHECK(mount("tmpfs", NSF_DIR, "tmpfs", MS_NOSYMFOLLOW, NULL));
}
END_SETUP()

FN_TEST(noexec_blocks_exec)
{
	exec_and_expect("/bin/sh", 0);

	copy_file("/bin/sh", NX_DIR "/sh");
	TEST_SUCC(chmod(NX_DIR "/sh", 0755));
	exec_and_expect(NX_DIR "/sh", EACCES);
}
END_TEST()

FN_TEST(nodev_blocks_device_open)
{
	int fd = TEST_SUCC(open("/dev/null", O_RDWR));
	TEST_SUCC(close(fd));

	TEST_SUCC(mknod(ND_DIR "/null", S_IFCHR | 0666, makedev(1, 3)));
	TEST_ERRNO(open(ND_DIR "/null", O_RDWR), EACCES);
}
END_TEST()

FN_TEST(nosymfollow_blocks_traversal)
{
	TEST_SUCC(symlink("/dev/null", "/tmp/mf_link_plain"));
	int fd = TEST_SUCC(open("/tmp/mf_link_plain", O_RDONLY));
	TEST_SUCC(close(fd));

	TEST_SUCC(symlink("/dev/null", NSF_DIR "/link"));
	char target[4096];
	TEST_RES(readlink(NSF_DIR "/link", target, sizeof(target) - 1),
		 _ret == 9 && memcmp(target, "/dev/null", 9) == 0);
	TEST_ERRNO(open(NSF_DIR "/link", O_RDONLY), ELOOP);
}
END_TEST()

FN_TEST(noexec_blocks_exec_mapping)
{
	int fd = TEST_SUCC(open(NX_DIR "/sh", O_RDONLY));

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_EXEC, MAP_PRIVATE, fd,
			0),
		   EPERM);

	// The same file maps without execute permission, but the mapping
	// cannot be made executable afterwards.
	void *addr =
		TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_PRIVATE, fd, 0));
	TEST_ERRNO(mprotect(addr, PAGE_SIZE, PROT_READ | PROT_EXEC), EACCES);
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(noexec_mapping_checks_access_first)
{
	static const struct {
		int open_flags;
		int prot;
		int map_flags;
		int expected_errno;
	} cases[] = {
		// O_PATH errors take precedence over noexec restrictions.
		{ O_PATH, PROT_NONE, MAP_PRIVATE, EBADF },
		{ O_PATH, PROT_EXEC, MAP_PRIVATE, EBADF },
		{ O_PATH, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_SHARED,
		  EBADF },
		{ O_PATH, PROT_READ | PROT_WRITE | PROT_EXEC,
		  MAP_SHARED_VALIDATE, EBADF },

		// The file must be opened readable in order to be mapped,
		// even with PROT_NONE.
		{ O_WRONLY, PROT_NONE, MAP_PRIVATE, EACCES },

		// The file must be opened readable in order to be mapped,
		// which takes precedence over the noexec restriction.
		{ O_WRONLY, PROT_EXEC, MAP_PRIVATE, EACCES },
		{ O_RDONLY, PROT_EXEC, MAP_PRIVATE, EPERM },

		// A shared PROT_WRITE mapping requires a writable file,
		// which takes precedence over the noexec restriction.
		{ O_RDONLY, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_SHARED,
		  EACCES },
		{ O_RDWR, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_SHARED,
		  EPERM },
		{ O_RDONLY, PROT_READ | PROT_WRITE | PROT_EXEC,
		  MAP_SHARED_VALIDATE, EACCES },
		{ O_RDWR, PROT_READ | PROT_WRITE | PROT_EXEC,
		  MAP_SHARED_VALIDATE, EPERM },
	};

	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		int fd = TEST_SUCC(open(NX_DIR "/sh", cases[i].open_flags));
		TEST_ERRNO(mmap(NULL, PAGE_SIZE, cases[i].prot,
				cases[i].map_flags, fd, 0),
			   cases[i].expected_errno);
		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(noexec_blocks_exec_access)
{
	TEST_ERRNO(access(NX_DIR "/sh", X_OK), EACCES);
	TEST_ERRNO(faccessat(AT_FDCWD, NX_DIR "/sh", X_OK, 0), EACCES);

	TEST_SUCC(access(NX_DIR "/sh", R_OK));
	TEST_SUCC(faccessat(AT_FDCWD, NX_DIR "/sh", R_OK, 0));
}
END_TEST()

FN_SETUP(remove_fixtures)
{
	CHECK_WITH(unlink("/tmp/mf_link_plain"), _ret == 0 || errno == ENOENT);
	CHECK_WITH(umount2(NX_DIR, MNT_DETACH), _ret == 0 || errno == EINVAL);
	CHECK_WITH(umount2(ND_DIR, MNT_DETACH), _ret == 0 || errno == EINVAL);
	CHECK_WITH(umount2(NSF_DIR, MNT_DETACH), _ret == 0 || errno == EINVAL);
	CHECK_WITH(rmdir(NX_DIR), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(ND_DIR), _ret == 0 || errno == ENOENT);
	CHECK_WITH(rmdir(NSF_DIR), _ret == 0 || errno == ENOENT);
}
END_SETUP()
