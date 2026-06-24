// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../../common/test.h"

#define PROP_ROOT "/tmp/mount_propagation"
#define PROP_RW PROP_ROOT "/rw"
#define PROP_RO PROP_ROOT "/ro"
#define PROP_SRC PROP_ROOT "/src"
#define PROP_NEWFS PROP_ROOT "/newfs"
#define PROP_RW_ROOTFS PROP_RW "/passthrough/cid/rootfs"
#define PROP_RO_ROOTFS PROP_RO "/passthrough/cid/rootfs"
#define PROP_MARKER "/marker"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void write_file(const char *path, const char *data)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644));
	CHECK(write(fd, data, strlen(data)));
	CHECK(close(fd));
}

static void read_file(const char *path, char *buf, size_t size)
{
	int fd = CHECK(open(path, O_RDONLY));

	ssize_t len = CHECK(read(fd, buf, size - 1));
	if (len >= 0) {
		buf[len] = '\0';
	}
	CHECK(close(fd));
}

static int mountinfo_has_path_and_dev(const char *path, dev_t dev)
{
	FILE *file = fopen("/proc/self/mountinfo", "r");
	CHECK_WITH(file != NULL ? 0 : -1, _ret == 0);

	char line[512];
	while (fgets(line, sizeof(line), file) != NULL) {
		unsigned int line_major;
		unsigned int line_minor;
		char mount_point[256];

		if (sscanf(line, "%*d %*d %u:%u %*s %255s", &line_major,
			   &line_minor, mount_point) == 3 &&
		    strcmp(mount_point, path) == 0 &&
		    line_major == major(dev) && line_minor == minor(dev)) {
			CHECK(fclose(file));
			return 1;
		}
	}

	CHECK(fclose(file));
	return 0;
}

static void ensure_kata_like_tree(void)
{
	CHECK(unshare(CLONE_NEWNS));

	ensure_dir(PROP_ROOT);
	ensure_dir(PROP_RW);
	ensure_dir(PROP_RO);
	ensure_dir(PROP_SRC);
	ensure_dir(PROP_NEWFS);

	CHECK(mount("tmpfs", PROP_RW, "tmpfs", 0, NULL));
	CHECK(mount(PROP_RW, PROP_RO, NULL, MS_BIND, NULL));
	CHECK(mount("none", PROP_RO, NULL, MS_SLAVE, NULL));

	ensure_dir(PROP_RW "/passthrough");
	ensure_dir(PROP_RW "/passthrough/cid");
	ensure_dir(PROP_RW_ROOTFS);
}

FN_TEST(bind_mount_propagates_to_slave_clone)
{
	char buf[32] = {};

	ensure_kata_like_tree();
	write_file(PROP_SRC PROP_MARKER, "bind-prop");

	TEST_SUCC(mount(PROP_SRC, PROP_RW_ROOTFS, NULL, MS_BIND, NULL));
	read_file(PROP_RO_ROOTFS PROP_MARKER, buf, sizeof(buf));
	TEST_RES(strcmp(buf, "bind-prop"), _ret == 0);
}
END_TEST()

FN_TEST(new_mount_propagates_to_slave_clone)
{
	char buf[32] = {};

	ensure_kata_like_tree();

	TEST_SUCC(mount("tmpfs", PROP_RW_ROOTFS, "tmpfs", 0, NULL));
	write_file(PROP_RW_ROOTFS PROP_MARKER, "newfs-prop");
	read_file(PROP_RO_ROOTFS PROP_MARKER, buf, sizeof(buf));
	TEST_RES(strcmp(buf, "newfs-prop"), _ret == 0);
}
END_TEST()

FN_TEST(propagated_bind_mount_is_visible_from_open_parent)
{
	char buf[32] = {};

	ensure_kata_like_tree();
	ensure_dir(PROP_SRC "/bin");
	write_file(PROP_SRC "/bin/sh", "openat-prop");

	int parent_fd = TEST_SUCC(
		open(PROP_RO "/passthrough/cid", O_RDONLY | O_DIRECTORY));
	TEST_SUCC(mount(PROP_SRC, PROP_RW_ROOTFS, NULL, MS_BIND, NULL));

	int sh_fd = TEST_SUCC(openat(parent_fd, "rootfs/bin/sh", O_RDONLY));
	TEST_SUCC(read(sh_fd, buf, sizeof(buf) - 1));
	TEST_RES(strcmp(buf, "openat-prop"), _ret == 0);
	TEST_SUCC(close(sh_fd));
	TEST_SUCC(close(parent_fd));
}
END_TEST()

FN_TEST(propagated_mount_changes_dev_and_mountinfo)
{
	struct stat parent_stat;
	struct stat rootfs_stat;

	ensure_kata_like_tree();

	TEST_SUCC(stat(PROP_RO "/passthrough/cid", &parent_stat));
	TEST_SUCC(mount("tmpfs", PROP_RW_ROOTFS, "tmpfs", 0, NULL));
	TEST_SUCC(stat(PROP_RO_ROOTFS, &rootfs_stat));

	TEST_RES(rootfs_stat.st_dev != parent_stat.st_dev, _ret);
	TEST_RES(mountinfo_has_path_and_dev(PROP_RO_ROOTFS, rootfs_stat.st_dev),
		 _ret == 1);
}
END_TEST()
