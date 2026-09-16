// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ol_rmdir_test"
#define LOWER_DIR TEST_DIR "/lower"
#define UPPER_DIR TEST_DIR "/upper"
#define WORK_DIR TEST_DIR "/work"
#define MERGED_DIR TEST_DIR "/merged"

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void write_file(const char *path)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));
	CHECK(close(fd));
}

static void setup_overlay_tree(void)
{
	create_dir(TEST_DIR);
	create_dir(LOWER_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(MERGED_DIR);

	/* 构造一个只在 lower 层存在的目录 D，里面放一个 whiteout 文件 */
	create_dir(LOWER_DIR "/D");
	write_file(LOWER_DIR "/D/.wh.foo");

	char options[256];
	snprintf(options, sizeof(options), "lowerdir=%s,upperdir=%s,workdir=%s",
		 LOWER_DIR, UPPER_DIR, WORK_DIR);
	CHECK(mount("overlay", MERGED_DIR, "overlay", 0, options));
}

static void cleanup_overlay_tree(void)
{
	/* Cleanup 阶段所有操作失败都无所谓，直接忽略错误 */

	umount(MERGED_DIR);

	/* 清理 lower 层 */
	unlink(LOWER_DIR "/D/.wh.foo");
	rmdir(LOWER_DIR "/D");

	/* 清理 upper 层：内核 rmdir 会在 upper 层留下 whiteout 文件 */
	unlink(UPPER_DIR "/.wh.D");
	rmdir(UPPER_DIR "/D");

	/* 清理剩余空目录 */
	rmdir(MERGED_DIR);
	rmdir(WORK_DIR);
	rmdir(UPPER_DIR);
	rmdir(LOWER_DIR);
	rmdir(TEST_DIR);
}

FN_SETUP(init)
{
	setup_overlay_tree();
}

END_SETUP()

FN_TEST(rmdir_lower_only)
{
	/* rmdir 一个只在 lower 层存在的目录，应该成功，而不是 panic */
	TEST_SUCC(rmdir(MERGED_DIR "/D"));

	/* 验证 D 是否从合并视图中消失 */
	struct stat st;
	TEST_ERRNO(stat(MERGED_DIR "/D", &st), ENOENT);
}

END_TEST()

FN_SETUP(cleanup)
{
	cleanup_overlay_tree();
}

END_SETUP()