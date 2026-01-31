// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/types.h>

#include "../../common/test.h"

static int get_dev_id(const char *path, dev_t *dev_id)
{
	struct stat st;

	if (stat(path, &st) < 0) {
		return -1;
	}

	*dev_id = st.st_dev;

	return 0;
}

FN_TEST(pseudo_fs_dev_id_major_is_zero)
{
	dev_t dev_id;

	TEST_RES(get_dev_id("/proc", &dev_id), major(dev_id) == 0);

	TEST_RES(get_dev_id("/sys", &dev_id), major(dev_id) == 0);

	TEST_RES(get_dev_id("/dev/pts", &dev_id), major(dev_id) == 0);

	TEST_RES(get_dev_id("/sys/fs/cgroup", &dev_id), major(dev_id) == 0);

	TEST_RES(get_dev_id("/sys/kernel/config", &dev_id), major(dev_id) == 0);

	TEST_RES(get_dev_id("/dev", &dev_id), major(dev_id) == 0);
}
END_TEST()

FN_TEST(pseudo_fs_dev_id_minor_is_positive)
{
	dev_t dev_id;

	TEST_RES(get_dev_id("/proc", &dev_id), minor(dev_id) > 0);

	TEST_RES(get_dev_id("/sys", &dev_id), minor(dev_id) > 0);

	TEST_RES(get_dev_id("/dev/pts", &dev_id), minor(dev_id) > 0);

	TEST_RES(get_dev_id("/sys/fs/cgroup", &dev_id), minor(dev_id) > 0);

	TEST_RES(get_dev_id("/sys/kernel/config", &dev_id), minor(dev_id) > 0);

	TEST_RES(get_dev_id("/dev", &dev_id), minor(dev_id) > 0);
}
END_TEST()

FN_TEST(pseudo_fs_dev_id_unique_per_mount)
{
	dev_t dev_ids[6];

	TEST_SUCC(get_dev_id("/proc", &dev_ids[0]));
	TEST_SUCC(get_dev_id("/sys", &dev_ids[1]));
	TEST_SUCC(get_dev_id("/dev/pts", &dev_ids[2]));
	TEST_SUCC(get_dev_id("/sys/fs/cgroup", &dev_ids[3]));
	TEST_SUCC(get_dev_id("/sys/kernel/config", &dev_ids[4]));
	TEST_SUCC(get_dev_id("/dev", &dev_ids[5]));

	// Test that each pseudo filesystem has a unique device ID
	for (int i = 0; i < 6; i++) {
		for (int j = i + 1; j < 6; j++) {
			TEST_RES(dev_ids[i] != dev_ids[j], _ret);
		}
	}
}
END_TEST()

FN_TEST(pseudo_fs_files_share_same_container_dev_id)
{
	dev_t dev_id_proc_self, dev_id_proc_cpuinfo;
	dev_t dev_id_sys_kernel, dev_id_sys_fs;

	// Files in the same pseudo fs should share the same device ID
	TEST_SUCC(get_dev_id("/proc/self", &dev_id_proc_self));
	TEST_SUCC(get_dev_id("/proc/cpuinfo", &dev_id_proc_cpuinfo));
	TEST_RES(dev_id_proc_self == dev_id_proc_cpuinfo, _ret);

	TEST_SUCC(get_dev_id("/sys/kernel", &dev_id_sys_kernel));
	TEST_SUCC(get_dev_id("/sys/fs", &dev_id_sys_fs));
	TEST_RES(dev_id_sys_kernel == dev_id_sys_fs, _ret);
}
END_TEST()
