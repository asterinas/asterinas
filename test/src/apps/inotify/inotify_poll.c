// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#include <sys/ioctl.h>

static void die(const char *msg)
{
	perror(msg);
	exit(1);
}

static void ensure_dir(const char *path)
{
	if (mkdir(path, 0700) < 0 && errno != EEXIST)
		die("mkdir");
}

static void touch_after_delay(const char *path)
{
	struct timespec ts = { .tv_sec = 0, .tv_nsec = 200 * 1000 * 1000 };
	nanosleep(&ts, NULL);
	int fd = open(path, O_CREAT | O_WRONLY, 0600);
	if (fd < 0)
		die("open");
	ssize_t wr = write(fd, "x", 1);
	if (wr < 0)
		die("write");
	if (wr != 1) {
		fprintf(stderr, "short write\n");
		exit(1);
	}
	close(fd);
}

int main(void)
{
	const char *dir = "inotify_tmp";
	const char *file = "inotify_tmp/testfile";

	ensure_dir(dir);

	int ifd = inotify_init1(0);
	if (ifd < 0)
		die("inotify_init1");

	int wd = inotify_add_watch(ifd, dir,
				   IN_CREATE | IN_MODIFY | IN_MOVED_TO);
	if (wd < 0)
		die("inotify_add_watch");

	/* Subtest 0: without events, poll(0) should timeout */
	{
		struct pollfd p0 = { .fd = ifd,
				     .events = POLLIN,
				     .revents = 0 };
		int r0 = poll(&p0, 1, 0);
		if (r0 < 0)
			die("poll");
		if (r0 != 0) {
			fprintf(stderr,
				"unexpected ready without events: revents=0x%x\n",
				p0.revents);
			return 4;
		}
	}

	pid_t pid = fork();
	if (pid < 0)
		die("fork");
	if (pid == 0) {
		touch_after_delay(file);
		_exit(0);
	}

	struct pollfd pfd = { .fd = ifd, .events = POLLIN, .revents = 0 };
	int pret = poll(&pfd, 1, 5000);
	if (pret < 0)
		die("poll");
	if (pret == 0) {
		fprintf(stderr, "poll timeout without events\n");
		return 2;
	}
	if (!(pfd.revents & POLLIN)) {
		fprintf(stderr, "unexpected revents: 0x%x\n", pfd.revents);
		return 3;
	}

	char buf[4096]
		__attribute__((aligned(__alignof__(struct inotify_event))));
	/* FIONREAD should indicate pending bytes before read */
	int pending = 0;
	if (ioctl(ifd, FIONREAD, &pending) < 0)
		die("ioctl(FIONREAD)");
	if (pending <= 0) {
		fprintf(stderr, "FIONREAD should be > 0 when POLLIN set\n");
		return 5;
	}
	ssize_t len = read(ifd, buf, sizeof(buf));
	if (len < 0)
		die("read");

	/* After drain, poll(0) should not fire */
	{
		struct pollfd p1 = { .fd = ifd,
				     .events = POLLIN,
				     .revents = 0 };
		int r1 = poll(&p1, 1, 0);
		if (r1 < 0)
			die("poll");
		if (r1 != 0) {
			fprintf(stderr,
				"still ready after drain: revents=0x%x\n",
				p1.revents);
			return 6;
		}
	}

	int status = 0;
	waitpid(pid, &status, 0);
	close(ifd);
	unlink(file);
	rmdir(dir);

	printf("inotify poll basic test: OK (bytes=%zd)\n", len);
	return 0;
}
