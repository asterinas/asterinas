// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/types.h>

#include "../../common/test.h"

struct dev_id {
	unsigned int major;
	unsigned int minor;
	dev_t st_dev;
};

static int get_dev_id(const char *path, struct dev_id *dev_id)
{
	struct stat st;

	if (stat(path, &st) < 0) {
		return -1;
	}

	dev_id->st_dev = st.st_dev;
	dev_id->major = major(st.st_dev);
	dev_id->minor = minor(st.st_dev);

	return 0;
}

static int dev_id_equal(const struct dev_id *a, const struct dev_id *b)
{
	return a->st_dev == b->st_dev;
}

FN_TEST(pseudo_fs_dev_id_major_is_zero)
{
	struct dev_id dev_id;

	TEST_SUCC(get_dev_id("/proc", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);

	TEST_SUCC(get_dev_id("/sys", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);

	TEST_SUCC(get_dev_id("/dev/pts", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);

	TEST_SUCC(get_dev_id("/sys/fs/cgroup", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);

	TEST_SUCC(get_dev_id("/sys/kernel/config", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);

	TEST_SUCC(get_dev_id("/dev", &dev_id));
	TEST_RES(dev_id.major, _ret == 0);
}
END_TEST()

FN_TEST(pseudo_fs_dev_id_minor_is_positive)
{
	struct dev_id dev_id;

	TEST_SUCC(get_dev_id("/proc", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);

	TEST_SUCC(get_dev_id("/sys", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);

	TEST_SUCC(get_dev_id("/dev/pts", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);

	TEST_SUCC(get_dev_id("/sys/fs/cgroup", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);

	TEST_SUCC(get_dev_id("/sys/kernel/config", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);

	TEST_SUCC(get_dev_id("/dev", &dev_id));
	TEST_RES(dev_id.minor, _ret > 0);
}
END_TEST()

FN_TEST(pseudo_fs_dev_id_unique_per_mount)
{
	struct dev_id dev_ids[6];

	TEST_SUCC(get_dev_id("/proc", &dev_ids[0]));
	TEST_SUCC(get_dev_id("/sys", &dev_ids[1]));
	TEST_SUCC(get_dev_id("/dev/pts", &dev_ids[2]));
	TEST_SUCC(get_dev_id("/sys/fs/cgroup", &dev_ids[3]));
	TEST_SUCC(get_dev_id("/sys/kernel/config", &dev_ids[4]));
	TEST_SUCC(get_dev_id("/dev", &dev_ids[5]));

	// Test that each pseudo filesystem has a unique device ID
	for (int i = 0; i < 6; i++) {
		for (int j = i + 1; j < 6; j++) {
			TEST_RES(!dev_id_equal(&dev_ids[i], &dev_ids[j]), _ret);
		}
	}
}
END_TEST()

FN_TEST(pseudo_fs_files_share_same_container_dev_id)
{
	struct dev_id dev_id_proc_self, dev_id_proc_cpuinfo;
	struct dev_id dev_id_sys_kernel, dev_id_sys_fs;

	// Files in the same pseudo fs should share the same device ID
	TEST_SUCC(get_dev_id("/proc/self", &dev_id_proc_self));
	TEST_SUCC(get_dev_id("/proc/cpuinfo", &dev_id_proc_cpuinfo));
	TEST_RES(dev_id_equal(&dev_id_proc_self, &dev_id_proc_cpuinfo), _ret);

	TEST_SUCC(get_dev_id("/sys/kernel", &dev_id_sys_kernel));
	TEST_SUCC(get_dev_id("/sys/fs", &dev_id_sys_fs));
	TEST_RES(dev_id_equal(&dev_id_sys_kernel, &dev_id_sys_fs), _ret);
}
END_TEST()
