// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>
#include "../../common/test.h"

#define DEVPTS_MOUNT "/tmp/devpts-peer"
#define PTMX_PATH DEVPTS_MOUNT "/ptmx"

static int master;
static char slave_path[128];

FN_SETUP(mount_devpts)
{
	CHECK(mkdir(DEVPTS_MOUNT, 0755));
	CHECK(mount("devpts", DEVPTS_MOUNT, "devpts", 0, NULL));
	master = CHECK(open(PTMX_PATH, O_RDWR));

	int index;
	CHECK(ioctl(master, TIOCGPTN, &index));
	memset(slave_path, 0, sizeof(slave_path));
	sprintf(slave_path, DEVPTS_MOUNT "/%d", index);

	int lock = 0;
	CHECK(ioctl(master, TIOCSPTLCK, &lock));
}
END_SETUP()

FN_TEST(open_peer_from_master_mount)
{
	int peer = TEST_SUCC(ioctl(master, TIOCGPTPEER, O_RDWR));
	struct stat peer_stat;
	struct stat path_stat;
	TEST_SUCC(fstat(peer, &peer_stat));
	TEST_RES(stat(slave_path, &path_stat),
		 peer_stat.st_dev == path_stat.st_dev &&
			 peer_stat.st_ino == path_stat.st_ino);
	TEST_SUCC(close(peer));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(master));
	CHECK(umount(DEVPTS_MOUNT));
	CHECK(rmdir(DEVPTS_MOUNT));
}
END_SETUP()
