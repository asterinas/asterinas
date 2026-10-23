// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

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

// Runs `path` as `sh -c "exit 0"` in a child process. The child reports a
// failed exec through its exit status; the parent turns that status into the
// expected `errno` check.
static void exec_and_expect(const char *path, int want_errno)
{
	pid_t pid = CHECK(fork());

	if (pid == 0) {
		execl(path, "sh", "-c", "exit 0", (char *)NULL);
		_exit(errno);
	}

	int status = 0;
	CHECK(waitpid(pid, &status, 0));
	CHECK_WITH(WIFEXITED(status) && WEXITSTATUS(status) == want_errno,
		   (printf("exec of %s: want errno %d, child status %d\n", path,
			   want_errno, status),
		    1));
}

FN_SETUP(mount_fixtures)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));
	CHECK(mkdir(NX_DIR, 0755) || errno == EEXIST);
	CHECK(mkdir(ND_DIR, 0755) || errno == EEXIST);
	CHECK(mkdir(NSF_DIR, 0755) || errno == EEXIST);
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
	// The same program bits on an executable mount must run.
	exec_and_expect("/bin/sh", 0);

	// The copy keeps the execute permission and is readable; the mount
	// makes it non-executable.
	copy_file("/bin/sh", NX_DIR "/sh");
	CHECK(chmod(NX_DIR "/sh", 0755));
	exec_and_expect(NX_DIR "/sh", EACCES);
}
END_TEST()

FN_TEST(nodev_blocks_device_open)
{
	// Creating the device node is allowed; opening it is what `nodev`
	// governs.
	TEST_SUCC(mknod(ND_DIR "/null", S_IFCHR | 0666, makedev(1, 3)));

	// The same device on a mount without `nodev` opens.
	int fd = TEST_SUCC(open("/dev/null", O_RDWR));
	TEST_SUCC(close(fd));

	TEST_ERRNO(open(ND_DIR "/null", O_RDWR), EACCES);
}
END_TEST()

FN_TEST(nosymfollow_blocks_traversal)
{
	// A symlink outside the `nosymfollow` mount resolves.
	TEST_SUCC(symlink("/dev/null", "/tmp/mf_link_plain"));
	int fd = TEST_SUCC(open("/tmp/mf_link_plain", O_RDONLY));
	TEST_SUCC(close(fd));

	// The same link on the `nosymfollow` mount does not resolve. The link
	// itself remains readable without following it.
	TEST_SUCC(symlink("/dev/null", NSF_DIR "/link"));

	char target[4096];
	ssize_t len = TEST_SUCC(
		readlink(NSF_DIR "/link", target, sizeof(target) - 1));
	target[len] = '\0';
	CHECK_WITH(strcmp(target, "/dev/null") == 0,
		   (printf("readlink returned %s\n", target), 1));

	TEST_ERRNO(open(NSF_DIR "/link", O_RDONLY), ELOOP);
}
END_TEST()
