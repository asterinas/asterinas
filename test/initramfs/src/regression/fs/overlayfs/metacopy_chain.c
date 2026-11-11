// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_metacopy_chain"

#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define ROOT BASE_DIR "/data"

#define MOUNT_OPTIONS ",metacopy=on,redirect_dir=on"

#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN (sizeof(LOWER_DATA) - 1)

static void build_relative_multi_metacopy_fixture(void)
{
	/* Upper and top hold metadata only; only the bottom holds the data. */
	create_dir(ROOT "/bottom/mc10");
	write_file(ROOT "/bottom/mc10/file", LOWER_DATA);
	create_dir(ROOT "/top/mc10");
	write_zero_file(ROOT "/top/mc10/file", LOWER_DATA_LEN);
	set_metacopy(ROOT "/top/mc10/file");
	create_dir(ROOT "/upper/mc10");
	write_zero_file(ROOT "/upper/mc10/file", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/mc10/file");
}

static void build_absolute_multi_metacopy_fixture(void)
{
	/* Only the bottom data file is real; both metacopy layers point at it. */
	create_dir(ROOT "/bottom/mc11");
	create_dir(ROOT "/bottom/mc11/p");
	write_file(ROOT "/bottom/mc11/p/file", LOWER_DATA);
	create_dir(ROOT "/top/mc11");
	create_dir(ROOT "/top/mc11/p");
	write_zero_file(ROOT "/top/mc11/p/file", LOWER_DATA_LEN);
	set_metacopy(ROOT "/top/mc11/p/file");
	create_dir(ROOT "/upper/mc11");
	write_zero_file(ROOT "/upper/mc11/mirror", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/mc11/mirror");
	set_redirect(ROOT "/upper/mc11/mirror", "/mc11/p/file");
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_multi_lower_tree_at(ROOT);

	build_relative_multi_metacopy_fixture();
	build_absolute_multi_metacopy_fixture();

	mount_overlay_multi_lower_at(ROOT, MOUNT_OPTIONS);
}

END_SETUP()

FN_TEST(mc10_relative_multi_metacopy)
{
	struct stat st;
	char buf[64];
	int fd;

	/* The first lower hit is metadata only; the data is one layer deeper. */
	TEST_RES(read_body(ROOT "/merged/mc10/file", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(lstat(ROOT "/merged/mc10/file", &st),
		 _ret == 0 && st.st_size == LOWER_DATA_LEN);

	/* A writable open completes the copy-up from the recorded data source. */
	fd = TEST_SUCC(open(ROOT "/merged/mc10/file", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ROOT "/upper/mc10/file"), _ret == 1);
	TEST_RES(read_body(ROOT "/upper/mc10/file", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	report_body("upper body", buf, (ssize_t)strlen(buf));
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(read_body(ROOT "/merged/mc10/file", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_TEST(mc11_absolute_multi_metacopy)
{
	char buf[64];
	int fd;

	/* The absolute data path must continue past the top metadata-only file. */
	TEST_RES(read_body(ROOT "/merged/mc11/mirror", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	fd = TEST_SUCC(open(ROOT "/merged/mc11/mirror", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(read_body(ROOT "/upper/mc11/mirror", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(metacopy_absent(ROOT "/upper/mc11/mirror"), _ret == 1);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(ROOT);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
