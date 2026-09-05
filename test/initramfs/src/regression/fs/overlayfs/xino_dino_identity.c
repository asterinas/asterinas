// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

/* Two separate tmpfs superblocks keep the mount non-samefs. */
#define LOWER_BASE "/ovl_xino_lower"
#define BASE_DIR "/ovl_xino_identity"
#define LOWER_DIR LOWER_BASE "/lower"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define MERGED_DIR BASE_DIR "/merged"

#define LOWER_CONTENT "lower-data"

#define DIRENT_BUF_SIZE 4096

struct linux_dirent64 {
	uint64_t d_ino;
	int64_t d_off;
	unsigned short d_reclen;
	unsigned char d_type;
	char d_name[];
};

struct merged_dinos {
	ino_t lfile;
	ino_t ufile;
	ino_t ldir;
	ino_t parent;
	int lfile_count;
	int ufile_count;
	int ldir_count;
	int parent_count;
};

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void write_file(const char *path, const char *content)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));
	CHECK(write(fd, content, strlen(content)));
	CHECK(close(fd));
}

static int scan_merged_dinos(struct merged_dinos *out)
{
	memset(out, 0, sizeof(*out));

	int fd = open(MERGED_DIR, O_RDONLY | O_DIRECTORY);
	if (fd < 0)
		return -1;

	char buf[DIRENT_BUF_SIZE];
	for (;;) {
		int nread = syscall(SYS_getdents64, fd, buf, sizeof(buf));
		if (nread < 0) {
			close(fd);
			return -1;
		}
		if (nread == 0)
			break;

		for (int pos = 0; pos < nread;) {
			struct linux_dirent64 *d =
				(struct linux_dirent64 *)(buf + pos);
			const char *name = d->d_name;

			if (strcmp(name, "lfile") == 0) {
				out->lfile = d->d_ino;
				out->lfile_count++;
			} else if (strcmp(name, "ufile") == 0) {
				out->ufile = d->d_ino;
				out->ufile_count++;
			} else if (strcmp(name, "ldir") == 0) {
				out->ldir = d->d_ino;
				out->ldir_count++;
			} else if (strcmp(name, "..") == 0) {
				out->parent = d->d_ino;
				out->parent_count++;
			}

			pos += d->d_reclen;
		}
	}

	if (close(fd) < 0)
		return -1;
	return 0;
}

static void setup_overlay_tree(void)
{
	create_dir(LOWER_BASE);
	create_dir(BASE_DIR);

	CHECK(mount("tmpfs", LOWER_BASE, "tmpfs", 0, NULL));
	CHECK(mount("tmpfs", BASE_DIR, "tmpfs", 0, NULL));

	create_dir(LOWER_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(MERGED_DIR);

	write_file(LOWER_DIR "/lfile", LOWER_CONTENT);
	create_dir(LOWER_DIR "/ldir");
	write_file(LOWER_DIR "/ldir/inner", "inner-data");
	write_file(UPPER_DIR "/ufile", "upper-data");
}

static void cleanup_overlay_tree(void)
{
	CHECK(unlink(UPPER_DIR "/ufile"));
	/* `lfile` reached the upper layer through copy-up. */
	CHECK(unlink(UPPER_DIR "/lfile"));

	CHECK(rmdir(MERGED_DIR));
	/* The mount leaves this staging dir behind; it survives unmount. */
	CHECK(rmdir(WORK_DIR "/work"));
	CHECK(rmdir(WORK_DIR));
	CHECK(rmdir(UPPER_DIR));

	CHECK(unlink(LOWER_DIR "/lfile"));
	CHECK(unlink(LOWER_DIR "/ldir/inner"));
	CHECK(rmdir(LOWER_DIR "/ldir"));
	CHECK(rmdir(LOWER_DIR));
}

static void mount_overlay(void)
{
	char options[256];
	snprintf(options, sizeof(options),
		 "lowerdir=%s,upperdir=%s,workdir=%s,xino=on", LOWER_DIR,
		 UPPER_DIR, WORK_DIR);

	CHECK(mount("overlay", MERGED_DIR, "overlay", 0, options));
}

FN_SETUP(init)
{
	setup_overlay_tree();
	mount_overlay();
}

END_SETUP()

FN_TEST(xino_publishes_uniform_dev_and_encoded_ino)
{
	struct stat st_lfile, st_ufile, st_ldir, st_lower_lfile;

	TEST_SUCC(stat(MERGED_DIR "/lfile", &st_lfile));
	TEST_SUCC(stat(MERGED_DIR "/ufile", &st_ufile));
	TEST_SUCC(stat(MERGED_DIR "/ldir", &st_ldir));
	TEST_SUCC(stat(LOWER_DIR "/lfile", &st_lower_lfile));

	TEST_RES(st_lfile.st_dev, _ret == st_ufile.st_dev);
	TEST_RES(st_ufile.st_dev, _ret == st_ldir.st_dev);
	TEST_RES(st_lfile.st_dev, _ret != st_lower_lfile.st_dev);

	TEST_RES(st_lfile.st_ino, _ret != st_lower_lfile.st_ino);
}

END_TEST()

FN_TEST(dino_matches_stat_and_is_stable)
{
	struct stat st_lfile, st_ufile, st_ldir, st_root;
	struct merged_dinos first_pass, second_pass;

	TEST_SUCC(stat(MERGED_DIR "/lfile", &st_lfile));
	TEST_SUCC(stat(MERGED_DIR "/ufile", &st_ufile));
	TEST_SUCC(stat(MERGED_DIR "/ldir", &st_ldir));
	TEST_SUCC(stat(MERGED_DIR, &st_root));

	TEST_SUCC(scan_merged_dinos(&first_pass));
	TEST_SUCC(scan_merged_dinos(&second_pass));

	TEST_RES(first_pass.lfile_count, _ret == 1);
	TEST_RES(first_pass.ufile_count, _ret == 1);
	TEST_RES(first_pass.ldir_count, _ret == 1);
	TEST_RES(first_pass.parent_count, _ret == 1);
	TEST_RES(second_pass.lfile_count, _ret == 1);
	TEST_RES(second_pass.ufile_count, _ret == 1);
	TEST_RES(second_pass.ldir_count, _ret == 1);
	TEST_RES(second_pass.parent_count, _ret == 1);

	TEST_RES(first_pass.lfile, _ret == st_lfile.st_ino);
	TEST_RES(first_pass.ufile, _ret == st_ufile.st_ino);
	TEST_RES(first_pass.ldir, _ret == st_ldir.st_ino);
	TEST_RES(first_pass.parent, _ret == st_root.st_ino);

	TEST_RES(second_pass.lfile, _ret == first_pass.lfile);
	TEST_RES(second_pass.ufile, _ret == first_pass.ufile);
	TEST_RES(second_pass.ldir, _ret == first_pass.ldir);
	TEST_RES(second_pass.parent, _ret == first_pass.parent);
}

END_TEST()

FN_TEST(ino_stable_across_copyup)
{
	char buf[16];
	struct stat st_before, st_after;

	TEST_SUCC(stat(MERGED_DIR "/lfile", &st_before));

	int fd = TEST_SUCC(open(MERGED_DIR "/lfile", O_WRONLY));
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_RES(write(fd, "H", 1), _ret == 1);
	TEST_SUCC(close(fd));

	TEST_SUCC(stat(MERGED_DIR "/lfile", &st_after));
	TEST_RES(st_after.st_ino, _ret == st_before.st_ino);
	TEST_RES(st_after.st_dev, _ret == st_before.st_dev);

	fd = TEST_SUCC(open(MERGED_DIR "/lfile", O_RDONLY));
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == strlen(LOWER_CONTENT));
	TEST_SUCC(close(fd));
	TEST_RES(memcmp(buf, "Hower-data", strlen("Hower-data")), _ret == 0);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
	CHECK(umount(BASE_DIR));
	CHECK(umount(LOWER_BASE));
	CHECK(rmdir(BASE_DIR));
	CHECK(rmdir(LOWER_BASE));
}

END_SETUP()
