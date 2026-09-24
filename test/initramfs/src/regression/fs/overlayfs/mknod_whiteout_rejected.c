// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_mknod_whiteout_rejected"

#include <sys/sysmacros.h>

#include "ovl_common.h"

#define LOWER_NAME "hidden"
#define LOWER_DATA "lower-data"
#define FORGED_NAME "forged"
#define ALLOWED_NAME "allowed"

static void setup_overlay_tree(void)
{
	create_overlay_dirs();

	write_file(LOWER_DIR "/" LOWER_NAME, LOWER_DATA);
}

static void cleanup_overlay_tree(void)
{
	CHECK_WITH(unlink(UPPER_DIR "/" ALLOWED_NAME),
		   _ret == 0 || errno == ENOENT);
	CHECK(unlink(LOWER_DIR "/" LOWER_NAME));

	remove_overlay_dirs();
	CHECK(rmdir(LOWER_DIR));
	CHECK(rmdir(BASE_DIR));
}

FN_SETUP(init)
{
	setup_overlay_tree();
	mount_overlay();
}

END_SETUP()

FN_TEST(mknod_whiteout_rejected)
{
	struct stat st;

	/* The lower name is visible before anything is attempted on it. */
	TEST_SUCC(lstat(MERGED_DIR "/" LOWER_NAME, &st));
	TEST_RES(st.st_size, _ret == (off_t)strlen(LOWER_DATA));

	/* A name the merged view already shows is not writable at all. */
	TEST_ERRNO(mknod(MERGED_DIR "/" LOWER_NAME, S_IFCHR | 0600,
			 makedev(0, 0)),
		   EEXIST);

	/* A raw 0:0 char device would read back as a whiteout, so it is refused. */
	TEST_ERRNO(mknod(MERGED_DIR "/" FORGED_NAME, S_IFCHR | 0600,
			 makedev(0, 0)),
		   EPERM);
	TEST_ERRNO(lstat(UPPER_DIR "/" FORGED_NAME, &st), ENOENT);

	/* A device the upper can really hold is still creatable. */
	TEST_SUCC(mknod(MERGED_DIR "/" ALLOWED_NAME, S_IFCHR | 0600,
			makedev(1, 3)));
	TEST_SUCC(lstat(MERGED_DIR "/" ALLOWED_NAME, &st));
	TEST_RES(st.st_mode & S_IFMT, _ret == S_IFCHR);
	TEST_RES(st.st_rdev, _ret == makedev(1, 3));

	/* The lower name survived the two refused attempts. */
	TEST_SUCC(lstat(MERGED_DIR "/" LOWER_NAME, &st));
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
