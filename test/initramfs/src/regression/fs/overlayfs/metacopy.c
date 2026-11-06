// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_metacopy_test"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define ON_BASE BASE_DIR "/on"
#define OFF_BASE BASE_DIR "/off"
#define REM_BASE BASE_DIR "/remount"

#define ON_OPTIONS ",metacopy=on,redirect_dir=on"
#define OFF_OPTIONS ",metacopy=off,redirect_dir=nofollow"

#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN 10

static const char malformed_short_record[3] = { 0, 3, 0 };
static const char malformed_version_record[4] = { 1, 4, 0, 0 };
static const char malformed_len_record[8] = {
	0, 4, 0, 0, 0xAA, 0xBB, 0xCC, 0xDD
};

/* Writes `value` as the metacopy xattr of `path` (see METACOPY_XATTR). */
static void set_metacopy_value(const char *path, const void *value, size_t size)
{
	set_xattr_value(path, METACOPY_XATTR, value, size);
}

/* Returns 1 when the metacopy xattr of `path` is the v0 record, 0 otherwise. */
static int metacopy_is_v0(const char *path)
{
	return xattr_matches(path, METACOPY_XATTR, metacopy_v0,
			     sizeof(metacopy_v0));
}

/*
 * Returns 1 when the metacopy xattr of `path` equals the `size` bytes at
 * `value`, 0 otherwise.
 */
static int metacopy_is(const char *path, const void *value, size_t size)
{
	return xattr_matches(path, METACOPY_XATTR, value, size);
}

/* Returns 1 when the upper file at `path` is a `size`-byte all-zero regular file. */
static int upper_is_zero_filled(const char *path, size_t size)
{
	struct stat st;
	char value[64];
	ssize_t written;
	int fd;
	size_t i;

	if (size > sizeof(value))
		return 0;
	if (lstat(path, &st) != 0 || !S_ISREG(st.st_mode) ||
	    (size_t)st.st_size != size) {
		errno = 0;
		return 0;
	}
	fd = open(path, O_RDONLY);
	if (fd < 0) {
		errno = 0;
		return 0;
	}
	written = read(fd, value, size);
	close(fd);
	errno = 0;
	if (written != (ssize_t)size)
		return 0;
	for (i = 0; i < size; i++) {
		if (value[i] != 0)
			return 0;
	}
	return 1;
}

static void build_metadata_copy_up_reads_lower_fixture(void)
{
	/* A chmod-only copy-up writes a record and keeps reading the lower data. */
	create_dir(ON_BASE "/lower/metadata_read");
	create_dir(ON_BASE "/lower/metadata_read/parent");
	write_file(ON_BASE "/lower/metadata_read/parent/file", LOWER_DATA);
}

static void build_writable_open_completes_data_fixture(void)
{
	/* A writable open fills the data and drops the metacopy record. */
	create_dir(ON_BASE "/lower/writable_open");
	write_file(ON_BASE "/lower/writable_open/file", LOWER_DATA);
}

static void build_truncate_paths_complete_data_fixture(void)
{
	/* Both truncate paths rewrite the file body and drop the record. */
	create_dir(ON_BASE "/lower/truncate_paths");
	write_file(ON_BASE "/lower/truncate_paths/truncate_file", LOWER_DATA);
	write_file(ON_BASE "/lower/truncate_paths/ftruncate_file", LOWER_DATA);
}

static void build_metacopy_off_rejects_record_fixture(void)
{
	/* With metacopy=off a record-bearing file is rejected with EPERM. */
	create_dir(OFF_BASE "/lower/off_mode");
	write_file(OFF_BASE "/lower/off_mode/record", LOWER_DATA);
	write_file(OFF_BASE "/lower/off_mode/plain", "plain");
	create_dir(OFF_BASE "/upper/off_mode");
	write_file(OFF_BASE "/upper/off_mode/record", "");
	set_metacopy(OFF_BASE "/upper/off_mode/record");
}

static void build_rename_redirect_and_data_authority_fixture(void)
{
	/* A renamed record keeps its redirect and reads lower until opened writable. */
	create_dir(ON_BASE "/lower/rename_data");
	create_dir(ON_BASE "/lower/rename_data/parent");
	write_file(ON_BASE "/lower/rename_data/parent/origin", LOWER_DATA);
}

static void build_link_writes_absolute_redirect_fixture(void)
{
	/* A hardlink from the upper side writes an absolute self-redirect. */
	create_dir(ON_BASE "/lower/hardlink");
	create_dir(ON_BASE "/lower/hardlink/source_parent");
	write_file(ON_BASE "/lower/hardlink/source_parent/file", LOWER_DATA);
	create_dir(ON_BASE "/lower/hardlink/target_parent");
}

static void build_readonly_and_writable_mappings_fixture(void)
{
	/* Read-only mappings read through the record; writable ones complete the data. */
	create_dir(ON_BASE "/lower/mmap");
	write_file(ON_BASE "/lower/mmap/file", LOWER_DATA);
}

static void build_data_copy_up_failure_keeps_metacopy_fixture(void)
{
	/* After the lower side changes behind the record, EIO keeps the record. */
	create_dir(REM_BASE "/lower/tampered");
	create_dir(REM_BASE "/lower/tampered/size_changed");
	write_file(REM_BASE "/lower/tampered/size_changed/file", LOWER_DATA);
	create_dir(REM_BASE "/lower/tampered/lower_removed");
	write_file(REM_BASE "/lower/tampered/lower_removed/orphan", LOWER_DATA);
}

static void build_malformed_record_eio_fixture(void)
{
	/* Malformed records are rejected with EIO, a legacy empty one with EPERM. */
	create_dir(OFF_BASE "/lower/malformed");
	create_dir(OFF_BASE "/upper/malformed");

	write_file(OFF_BASE "/lower/malformed/too_short", LOWER_DATA);
	write_file(OFF_BASE "/upper/malformed/too_short", "");
	set_metacopy_value(OFF_BASE "/upper/malformed/too_short",
			   malformed_short_record,
			   sizeof(malformed_short_record));

	write_file(OFF_BASE "/lower/malformed/bad_version", LOWER_DATA);
	write_file(OFF_BASE "/upper/malformed/bad_version", "");
	set_metacopy_value(OFF_BASE "/upper/malformed/bad_version",
			   malformed_version_record,
			   sizeof(malformed_version_record));

	write_file(OFF_BASE "/lower/malformed/bad_len", LOWER_DATA);
	write_file(OFF_BASE "/upper/malformed/bad_len", "");
	set_metacopy_value(OFF_BASE "/upper/malformed/bad_len",
			   malformed_len_record, sizeof(malformed_len_record));

	write_file(OFF_BASE "/lower/malformed/legacy_empty", LOWER_DATA);
	write_file(OFF_BASE "/upper/malformed/legacy_empty", "");
	set_metacopy_value(OFF_BASE "/upper/malformed/legacy_empty", "", 0);
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_overlay_tree_at(ON_BASE);
	create_overlay_tree_at(OFF_BASE);
	create_overlay_tree_at(REM_BASE);

	build_metadata_copy_up_reads_lower_fixture();
	build_writable_open_completes_data_fixture();
	build_truncate_paths_complete_data_fixture();
	build_metacopy_off_rejects_record_fixture();
	build_rename_redirect_and_data_authority_fixture();
	build_link_writes_absolute_redirect_fixture();
	build_readonly_and_writable_mappings_fixture();
	build_data_copy_up_failure_keeps_metacopy_fixture();
	build_malformed_record_eio_fixture();

	mount_overlay_at(ON_BASE, ON_OPTIONS);
	mount_overlay_at(OFF_BASE, OFF_OPTIONS);
	mount_overlay_at(REM_BASE, ON_OPTIONS);
}

END_SETUP()

FN_TEST(mc01_metadata_copy_up_reads_lower)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(chmod(ON_BASE "/merged/metadata_read/parent/file", 0640));
	/*
	 * The writer's encoding; readers accept the empty form too, so this is
	 * the one exact-byte assertion on a record this mount wrote.
	 */
	TEST_RES(metacopy_is_v0(ON_BASE "/upper/metadata_read/parent/file"),
		 _ret == 1);
	TEST_RES(upper_is_zero_filled(ON_BASE
				      "/upper/metadata_read/parent/file",
				      LOWER_DATA_LEN),
		 _ret == 1);
	TEST_RES(read_body(ON_BASE "/merged/metadata_read/parent/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(read_body(ON_BASE "/lower/metadata_read/parent/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(lstat(ON_BASE "/merged/metadata_read/parent/file", &st),
		 _ret == 0 && st.st_size == LOWER_DATA_LEN);
}

END_TEST()

FN_TEST(mc02_writable_open_completes_data)
{
	char buf[64];
	int fd;

	TEST_SUCC(chmod(ON_BASE "/merged/writable_open/file", 0640));
	fd = TEST_SUCC(open(ON_BASE "/merged/writable_open/file", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_BASE "/upper/writable_open/file"),
		 _ret == 1);
	TEST_RES(read_body(ON_BASE "/upper/writable_open/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(read_body(ON_BASE "/merged/writable_open/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	fd = TEST_SUCC(open(ON_BASE "/merged/writable_open/file", O_WRONLY));
	TEST_SUCC(write(fd, "U", 1));
	TEST_SUCC(close(fd));
	TEST_RES(read_body(ON_BASE "/merged/writable_open/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, "Uower-data"), _ret == 0);
}

END_TEST()

FN_TEST(mc03_truncate_paths_complete_data)
{
	struct stat st;
	char buf[64];
	int fd;

	TEST_SUCC(chmod(ON_BASE "/merged/truncate_paths/truncate_file", 0640));
	TEST_SUCC(chmod(ON_BASE "/merged/truncate_paths/ftruncate_file", 0640));

	fd = TEST_SUCC(open(ON_BASE "/merged/truncate_paths/truncate_file",
			    O_WRONLY | O_TRUNC));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_BASE "/upper/truncate_paths/truncate_file"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/merged/truncate_paths/truncate_file", &st),
		 _ret == 0 && st.st_size == 0);
	TEST_RES(read_body(ON_BASE "/merged/truncate_paths/truncate_file", buf,
			   sizeof(buf)),
		 _ret == 0);
	TEST_RES(strcmp(buf, ""), _ret == 0);

	fd = TEST_SUCC(
		open(ON_BASE "/merged/truncate_paths/ftruncate_file", O_RDWR));
	TEST_SUCC(ftruncate(fd, 4));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_BASE
				 "/upper/truncate_paths/ftruncate_file"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/merged/truncate_paths/ftruncate_file", &st),
		 _ret == 0 && st.st_size == 4);
	TEST_RES(read_body(ON_BASE "/merged/truncate_paths/ftruncate_file", buf,
			   sizeof(buf)),
		 _ret == 4);
	TEST_RES(strcmp(buf, "lowe"), _ret == 0);

	TEST_RES(read_body(ON_BASE "/lower/truncate_paths/truncate_file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(read_body(ON_BASE "/lower/truncate_paths/ftruncate_file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_TEST(mc04_metacopy_off_rejects_record)
{
	struct stat st;
	char buf[64];

	TEST_ERRNO(lstat(OFF_BASE "/merged/off_mode/record", &st), EPERM);
	TEST_ERRNO(open(OFF_BASE "/merged/off_mode/record", O_RDONLY), EPERM);
	TEST_ERRNO(open(OFF_BASE "/merged/off_mode/record", O_RDWR), EPERM);
	TEST_RES(metacopy_present(OFF_BASE "/upper/off_mode/record"),
		 _ret == 1);
	TEST_RES(read_body(OFF_BASE "/merged/off_mode/plain", buf, sizeof(buf)),
		 _ret == 5);
	TEST_RES(strcmp(buf, "plain"), _ret == 0);
	TEST_RES(read_body(OFF_BASE "/lower/off_mode/record", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_TEST(mc05_rename_redirect_and_data_authority)
{
	struct stat st;
	char buf[64];
	int fd;

	TEST_SUCC(chmod(ON_BASE "/merged/rename_data/parent/origin", 0640));
	TEST_SUCC(rename(ON_BASE "/merged/rename_data/parent/origin",
			 ON_BASE "/merged/rename_data/parent/renamed"));
	TEST_RES(xattr_matches(ON_BASE "/upper/rename_data/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
	TEST_RES(metacopy_present(ON_BASE "/upper/rename_data/parent/renamed"),
		 _ret == 1);
	TEST_RES(read_body(ON_BASE "/merged/rename_data/parent/renamed", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_ERRNO(lstat(ON_BASE "/merged/rename_data/parent/origin", &st),
		   ENOENT);

	fd = TEST_SUCC(
		open(ON_BASE "/merged/rename_data/parent/renamed", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_BASE "/upper/rename_data/parent/renamed"),
		 _ret == 1);
	TEST_RES(xattr_matches(ON_BASE "/upper/rename_data/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
	TEST_RES(read_body(ON_BASE "/merged/rename_data/parent/renamed", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_TEST(mc06_link_writes_absolute_redirect)
{
	struct stat st_src, st_dst;
	char buf[64];
	int fd;

	TEST_SUCC(chmod(ON_BASE "/merged/hardlink/source_parent/file", 0640));
	TEST_SUCC(link(ON_BASE "/merged/hardlink/source_parent/file",
		       ON_BASE "/merged/hardlink/target_parent/link"));
	TEST_RES(xattr_matches(ON_BASE "/upper/hardlink/source_parent/file",
			       REDIRECT_XATTR, "/hardlink/source_parent/file",
			       strlen("/hardlink/source_parent/file")),
		 _ret == 1);
	TEST_RES(metacopy_present(ON_BASE "/upper/hardlink/source_parent/file"),
		 _ret == 1);
	TEST_SUCC(
		lstat(ON_BASE "/merged/hardlink/source_parent/file", &st_src));
	TEST_SUCC(
		lstat(ON_BASE "/merged/hardlink/target_parent/link", &st_dst));
	TEST_RES(st_src.st_ino == st_dst.st_ino, _ret == 1);
	TEST_RES(read_body(ON_BASE "/merged/hardlink/target_parent/link", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	fd = TEST_SUCC(
		open(ON_BASE "/merged/hardlink/target_parent/link", O_RDWR));
	TEST_SUCC(write(fd, "X", 1));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_BASE "/upper/hardlink/source_parent/file"),
		 _ret == 1);
	TEST_RES(read_body(ON_BASE "/merged/hardlink/source_parent/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, "Xower-data"), _ret == 0);
}

END_TEST()

FN_TEST(mc07_readonly_and_writable_mappings)
{
	char buf[64];
	void *mapping;
	int fd_ro, fd_rw;

	TEST_SUCC(chmod(ON_BASE "/merged/mmap/file", 0640));

	fd_ro = TEST_SUCC(open(ON_BASE "/merged/mmap/file", O_RDONLY));
	mapping = TEST_RES(mmap(NULL, LOWER_DATA_LEN, PROT_READ, MAP_SHARED,
				fd_ro, 0),
			   _ret != MAP_FAILED);
	TEST_RES(memcmp(mapping, LOWER_DATA, LOWER_DATA_LEN), _ret == 0);
	TEST_SUCC(munmap(mapping, LOWER_DATA_LEN));

	fd_rw = TEST_SUCC(open(ON_BASE "/merged/mmap/file", O_RDWR));
	TEST_RES(metacopy_absent(ON_BASE "/upper/mmap/file"), _ret == 1);
	mapping = TEST_RES(mmap(NULL, LOWER_DATA_LEN, PROT_READ | PROT_WRITE,
				MAP_SHARED, fd_rw, 0),
			   _ret != MAP_FAILED);
	((char *)mapping)[0] = 'U';
	TEST_RES(memcmp(mapping, "Uower-data", LOWER_DATA_LEN), _ret == 0);
	TEST_SUCC(munmap(mapping, LOWER_DATA_LEN));

	TEST_RES(pread(fd_rw, buf, LOWER_DATA_LEN, 0), _ret == LOWER_DATA_LEN);
	TEST_RES(memcmp(buf, "Uower-data", LOWER_DATA_LEN), _ret == 0);

	mapping = TEST_RES(mmap(NULL, LOWER_DATA_LEN, PROT_READ, MAP_SHARED,
				fd_ro, 0),
			   _ret != MAP_FAILED);
	TEST_RES(memcmp(mapping, "Uower-data", LOWER_DATA_LEN), _ret == 0);
	TEST_SUCC(munmap(mapping, LOWER_DATA_LEN));

	TEST_ERRNO(mmap(NULL, LOWER_DATA_LEN, PROT_WRITE, MAP_SHARED, fd_ro, 0),
		   EACCES);

	TEST_SUCC(close(fd_rw));
	TEST_SUCC(close(fd_ro));
}

END_TEST()

FN_TEST(mc08_data_copy_up_failure_keeps_metacopy)
{
	char buf[64];

	TEST_SUCC(chmod(REM_BASE "/merged/tampered/size_changed/file", 0640));
	TEST_SUCC(
		chmod(REM_BASE "/merged/tampered/lower_removed/orphan", 0640));
	unmount_overlay_at(REM_BASE);

	TEST_SUCC(truncate(REM_BASE "/upper/tampered/size_changed/file", 1));
	TEST_SUCC(unlink(REM_BASE "/lower/tampered/lower_removed/orphan"));

	mount_overlay_at(REM_BASE, ON_OPTIONS);

	TEST_RES(read_body(REM_BASE "/merged/tampered/size_changed/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_ERRNO(open(REM_BASE "/merged/tampered/size_changed/file",
			O_WRONLY),
		   EIO);
	TEST_RES(metacopy_present(REM_BASE "/upper/tampered/size_changed/file"),
		 _ret == 1);
	TEST_RES(read_body(REM_BASE "/merged/tampered/size_changed/file", buf,
			   sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_ERRNO(open(REM_BASE "/merged/tampered/lower_removed/orphan",
			O_RDONLY),
		   EIO);
	TEST_RES(metacopy_present(REM_BASE
				  "/upper/tampered/lower_removed/orphan"),
		 _ret == 1);
	TEST_RES(read_body(REM_BASE "/merged/tampered/lower_removed/orphan",
			   buf, sizeof(buf)),
		 _ret < 0);
}

END_TEST()

FN_TEST(mc09_malformed_record_eio)
{
	struct stat st;
	char buf[64];

	TEST_ERRNO(lstat(OFF_BASE "/merged/malformed/too_short", &st), EIO);
	TEST_ERRNO(open(OFF_BASE "/merged/malformed/too_short", O_RDONLY), EIO);
	TEST_ERRNO(lstat(OFF_BASE "/merged/malformed/bad_version", &st), EIO);
	TEST_ERRNO(open(OFF_BASE "/merged/malformed/bad_version", O_RDONLY),
		   EIO);
	TEST_ERRNO(lstat(OFF_BASE "/merged/malformed/bad_len", &st), EIO);
	TEST_ERRNO(open(OFF_BASE "/merged/malformed/bad_len", O_RDONLY), EIO);
	TEST_ERRNO(lstat(OFF_BASE "/merged/malformed/legacy_empty", &st),
		   EPERM);
	TEST_ERRNO(open(OFF_BASE "/merged/malformed/legacy_empty", O_RDONLY),
		   EPERM);

	TEST_RES(metacopy_is(OFF_BASE "/upper/malformed/too_short",
			     malformed_short_record,
			     sizeof(malformed_short_record)),
		 _ret == 1);
	TEST_RES(metacopy_is(OFF_BASE "/upper/malformed/bad_version",
			     malformed_version_record,
			     sizeof(malformed_version_record)),
		 _ret == 1);
	TEST_RES(metacopy_is(OFF_BASE "/upper/malformed/bad_len",
			     malformed_len_record,
			     sizeof(malformed_len_record)),
		 _ret == 1);
	TEST_RES(xattr_matches(OFF_BASE "/upper/malformed/legacy_empty",
			       METACOPY_XATTR, "", 0),
		 _ret == 1);

	TEST_RES(read_body(OFF_BASE "/merged/malformed/too_short", buf,
			   sizeof(buf)),
		 _ret < 0);
	TEST_RES(read_body(OFF_BASE "/merged/malformed/bad_version", buf,
			   sizeof(buf)),
		 _ret < 0);
	TEST_RES(read_body(OFF_BASE "/merged/malformed/bad_len", buf,
			   sizeof(buf)),
		 _ret < 0);
	TEST_RES(read_body(OFF_BASE "/merged/malformed/legacy_empty", buf,
			   sizeof(buf)),
		 _ret < 0);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(REM_BASE);
	unmount_overlay_at(OFF_BASE);
	unmount_overlay_at(ON_BASE);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
