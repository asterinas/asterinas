// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_chain_edge"

#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

/* Two lower layers (top, bottom): n1-n4, n6, n8, n9. */
#define ROOT BASE_DIR "/two"

/* Three lower layers (t1, t2, bottom): n5, n7. */
#define ROOT3 BASE_DIR "/three"

/* The metacopy cases fabricate records, which a mount must follow. */
#define MOUNT_OPTIONS ",metacopy=on,redirect_dir=on"

#define DEEP_DATA "deep-data"
#define DEEP_DATA_LEN (sizeof(DEEP_DATA) - 1)
#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN (sizeof(LOWER_DATA) - 1)

static void build_absolute_self_loop_fixture(void)
{
	/*
	 * The upper absolute redirect enters the walk; the top loop keeps
	 * rewriting the last component onto itself.
	 */
	create_dir(ROOT "/upper/loop");
	set_redirect(ROOT "/upper/loop", "/loop/self");
	create_dir(ROOT "/top/loop");
	create_dir(ROOT "/top/loop/self");
	set_redirect(ROOT "/top/loop/self", "/loop/self");
	/* No /bottom/loop: the cycle never needs the deepest layer. */
}

static void build_two_hop_absolute_cycle_fixture(void)
{
	/* Both hops live in the top layer, so the count is per rewrite. */
	create_dir(ROOT "/upper/cycle");
	set_redirect(ROOT "/upper/cycle", "/cycle/a");
	create_dir(ROOT "/top/cycle");
	create_dir(ROOT "/top/cycle/a");
	set_redirect(ROOT "/top/cycle/a", "/cycle/b");
	create_dir(ROOT "/top/cycle/b");
	set_redirect(ROOT "/top/cycle/b", "/cycle/a");
}

static void build_relative_self_redirect_fixture(void)
{
	/* A relative value equal to the component replaces nothing. */
	create_dir(ROOT "/upper/selfref");
	set_redirect(ROOT "/upper/selfref", "/selfref/dir");
	create_dir(ROOT "/top/selfref");
	create_dir(ROOT "/top/selfref/dir");
	set_redirect(ROOT "/top/selfref/dir", "dir");
}

static void build_opaque_inside_redirect_fixture(void)
{
	/* An opaque directory in the middle of the path stops the deeper layer. */
	create_dir(ROOT "/upper/opq");
	create_dir(ROOT "/upper/opq/alias");
	set_redirect(ROOT "/upper/opq/alias", "/opq/link/c");
	create_dir(ROOT "/top/opq");
	create_dir(ROOT "/top/opq/link");
	set_opaque(ROOT "/top/opq/link");
	create_dir(ROOT "/top/opq/link/c");
	write_file(ROOT "/top/opq/link/c/top-only", "t");
	create_dir(ROOT "/bottom/opq");
	create_dir(ROOT "/bottom/opq/link");
	create_dir(ROOT "/bottom/opq/link/c");
	write_file(ROOT "/bottom/opq/link/c/bottom-only", "b");
}

static void build_relative_then_absolute_fixture(void)
{
	/*
	 * The walk meets a relative hop and then an absolute one; the carrier
	 * top/mix/b holds a child of its own so its retention is observable.
	 */
	create_dir(ROOT "/upper/mix");
	create_dir(ROOT "/upper/mix/alias");
	set_redirect(ROOT "/upper/mix/alias", "/mix/a");
	create_dir(ROOT "/top/mix");
	create_dir(ROOT "/top/mix/a");
	set_redirect(ROOT "/top/mix/a", "b");
	create_dir(ROOT "/top/mix/b");
	set_redirect(ROOT "/top/mix/b", "/mix/c");
	write_file(ROOT "/top/mix/b/carrier-own", "c");
	create_dir(ROOT "/bottom/mix");
	create_dir(ROOT "/bottom/mix/c");
	write_file(ROOT "/bottom/mix/c/file", DEEP_DATA);
}

static void build_metacopy_before_non_regular_fixture(void)
{
	/* The data path may only publish a regular file without a record. */
	create_dir(ROOT "/upper/m4");
	write_zero_file(ROOT "/upper/m4/mirror", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/m4/mirror");
	set_redirect(ROOT "/upper/m4/mirror", "/m4/p/file");
	create_dir(ROOT "/top/m4");
	create_dir(ROOT "/top/m4/p");
	write_zero_file(ROOT "/top/m4/p/file", LOWER_DATA_LEN);
	set_metacopy(ROOT "/top/m4/p/file");
	create_dir(ROOT "/bottom/m4");
	create_dir(ROOT "/bottom/m4/p");
	create_dir(ROOT "/bottom/m4/p/file");
}

static void build_redirect_cycle_on_data_path_fixture(void)
{
	/* The data walk rewrites onto a directory that redirects to itself. */
	create_dir(ROOT "/upper/dt");
	write_zero_file(ROOT "/upper/dt/mirror", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/dt/mirror");
	set_redirect(ROOT "/upper/dt/mirror", "/dt/a/file");
	create_dir(ROOT "/top/dt");
	create_dir(ROOT "/top/dt/a");
	set_redirect(ROOT "/top/dt/a", "/dt/a");
}

static void build_three_layer_metacopy_fixture(void)
{
	/* Two consecutive metadata-only layers before the layer with data. */
	create_dir(ROOT3 "/upper/m3");
	write_zero_file(ROOT3 "/upper/m3/file", LOWER_DATA_LEN);
	set_metacopy(ROOT3 "/upper/m3/file");
	create_dir(ROOT3 "/t1/m3");
	write_zero_file(ROOT3 "/t1/m3/file", LOWER_DATA_LEN);
	set_metacopy(ROOT3 "/t1/m3/file");
	create_dir(ROOT3 "/t2/m3");
	write_zero_file(ROOT3 "/t2/m3/file", LOWER_DATA_LEN);
	set_metacopy(ROOT3 "/t2/m3/file");
	create_dir(ROOT3 "/bottom/m3");
	write_file(ROOT3 "/bottom/m3/file", LOWER_DATA);
}

static void build_whiteout_below_walked_layer_fixture(void)
{
	/* The whiteout sits one layer below the one that already landed. */
	create_dir(ROOT3 "/upper/wt");
	create_dir(ROOT3 "/upper/wt/alias");
	set_redirect(ROOT3 "/upper/wt/alias", "/wt/a");
	create_dir(ROOT3 "/t1/wt");
	create_dir(ROOT3 "/t1/wt/a");
	write_file(ROOT3 "/t1/wt/a/top-own", "t");
	create_dir(ROOT3 "/t2/wt");
	create_whiteout(ROOT3 "/t2/wt/a");
	create_dir(ROOT3 "/bottom/wt");
	create_dir(ROOT3 "/bottom/wt/a");
	write_file(ROOT3 "/bottom/wt/a/deep-own", "d");
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_multi_lower_tree_at(ROOT);

	build_absolute_self_loop_fixture();
	build_two_hop_absolute_cycle_fixture();
	build_relative_self_redirect_fixture();
	build_opaque_inside_redirect_fixture();
	build_relative_then_absolute_fixture();
	build_metacopy_before_non_regular_fixture();
	build_redirect_cycle_on_data_path_fixture();

	mount_overlay_multi_lower_at(ROOT, MOUNT_OPTIONS);

	create_three_lower_tree_at(ROOT3);

	build_three_layer_metacopy_fixture();
	build_whiteout_below_walked_layer_fixture();

	mount_overlay_three_lower_at(ROOT3, MOUNT_OPTIONS);
}

END_SETUP()

FN_TEST(n1_absolute_self_loop)
{
	struct stat st;

	/* A directory redirecting to its own path must not spin the kernel. */
	TEST_ERRNO(lstat(ROOT "/merged/loop", &st), ELOOP);
	TEST_ERRNO(open(ROOT "/merged/loop", O_RDONLY), ELOOP);
}

END_TEST()

FN_TEST(n2_two_hop_absolute_cycle)
{
	struct stat st;

	/* Both hops are in layer 1, so the budget counts rewrites, not layers. */
	TEST_ERRNO(lstat(ROOT "/merged/cycle", &st), ELOOP);
}

END_TEST()

FN_TEST(n3_relative_self_redirect)
{
	struct stat st;

	/*
	 * A relative value equal to the component it replaces is a no-progress
	 * rewrite: the lookup succeeds, consumes no budget and sees no entries.
	 */
	TEST_SUCC(lstat(ROOT "/merged/selfref", &st));
	TEST_RES(st.st_mode & S_IFMT, _ret == S_IFDIR);
	TEST_RES(count_entries(ROOT "/merged/selfref"), _ret == 0);
}

END_TEST()

FN_TEST(n4_opaque_inside_redirect_path)
{
	/* The merged walk keeps the opaque barrier in the middle of the path. */
	TEST_RES(find_entry(ROOT "/merged/opq/alias", "top-only"), _ret == 1);
	TEST_RES(find_entry(ROOT "/merged/opq/alias", "bottom-only"),
		 _ret == 0);
}

END_TEST()

FN_TEST(n5_three_layer_metacopy)
{
	struct stat st;
	char buf[64];
	int fd;

	/* Two metadata-only layers are skipped before the data is accepted. */
	TEST_RES(read_body(ROOT3 "/merged/m3/file", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
	TEST_RES(lstat(ROOT3 "/merged/m3/file", &st),
		 _ret == 0 && st.st_size == LOWER_DATA_LEN);

	/* A writable open completes the copy-up from the recorded data source. */
	fd = TEST_SUCC(open(ROOT3 "/merged/m3/file", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ROOT3 "/upper/m3/file"), _ret == 1);
	TEST_RES(read_body(ROOT3 "/upper/m3/file", buf, sizeof(buf)),
		 _ret == LOWER_DATA_LEN);
	report_body("upper body", buf, (ssize_t)strlen(buf));
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_TEST(n6_relative_then_absolute)
{
	struct stat st;
	char buf[64];

	/* A relative hop and then an absolute one, in one walk. */
	TEST_RES(read_body(ROOT "/merged/mix/alias/file", buf, sizeof(buf)),
		 _ret == DEEP_DATA_LEN);
	TEST_RES(strcmp(buf, DEEP_DATA), _ret == 0);
	TEST_RES(lstat(ROOT "/merged/mix/alias/file", &st),
		 _ret == 0 && st.st_size == DEEP_DATA_LEN);
	TEST_RES(find_entry(ROOT "/merged/mix/alias", "file"), _ret == 1);

	/*
	 * The carrier top/mix/b stays in the layer stack, so its own child is
	 * visible in the merged view of the redirecting name.
	 */
	TEST_RES(find_entry(ROOT "/merged/mix/alias", "carrier-own"),
		 _ret == 1);
}

END_TEST()

FN_TEST(n7_whiteout_below_walked_layer)
{
	/* The landed top layer survives; the whiteouted deeper layer is barred. */
	TEST_RES(find_entry(ROOT3 "/merged/wt/alias", "top-own"), _ret == 1);
	TEST_RES(find_entry(ROOT3 "/merged/wt/alias", "deep-own"), _ret == 0);
}

END_TEST()

FN_TEST(n8_metacopy_before_non_regular)
{
	struct stat st;

	/* A metadata-only hop followed by a directory publishes no object. */
	TEST_ERRNO(lstat(ROOT "/merged/m4/mirror", &st), EIO);
	TEST_ERRNO(open(ROOT "/merged/m4/mirror", O_RDONLY), EIO);
}

END_TEST()

FN_TEST(n9_redirect_cycle_on_data_path)
{
	/* Exhausting the budget on the data path is ELOOP, never EIO. */
	TEST_ERRNO(open(ROOT "/merged/dt/mirror", O_RDONLY), ELOOP);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(ROOT3);
	unmount_overlay_at(ROOT);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
