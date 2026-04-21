// SPDX-License-Identifier: MPL-2.0

#include <sys/stat.h>
#include <sys/sysmacros.h>
#include "../common/test.h"

/*
 * Device-node modes should normally be covered by the test for each device.
 * This test covers devtmpfs modes for devices whose tests do not yet exist.
 */

struct device_node {
	const char *path;
	mode_t mode;
	unsigned int major;
	unsigned int minor;
};

static const struct device_node device_nodes[] = {
	/* Explicit Linux overrides (non-default mode). */
	{ "/dev/null", 0666, 1, 3 },
	{ "/dev/zero", 0666, 1, 5 },
	{ "/dev/tty", 0666, 5, 0 },
	/*
	 * Devices that fall back to the kernel devtmpfs default (0600).
	 *
	 * Do not replace these modes with values observed on a host Linux.
	 * Systemd may later change the default modes of these devices in user space.
	 * Reference: <https://github.com/systemd/systemd/blob/9149c7595305a7c4d105d5d33ba25733af4302eb/rules.d/50-udev-default.rules.in>
	 */
	{ "/dev/console", 0600, 5, 1 },
	{ "/dev/tty0", 0600, 4, 0 },
	{ "/dev/tty1", 0600, 4, 1 },
	{ "/dev/ttyS0", 0600, 4, 64 },
};

FN_TEST(rdev_and_mode)
{
	for (size_t i = 0; i < sizeof(device_nodes) / sizeof(device_nodes[0]);
	     i++) {
		const struct device_node *node = &device_nodes[i];
		struct stat stat_buf;

		TEST_RES(stat(node->path, &stat_buf),
			 S_ISCHR(stat_buf.st_mode) &&
				 stat_buf.st_rdev ==
					 makedev(node->major, node->minor) &&
				 (stat_buf.st_mode & 0777) == node->mode);
	}
}
END_TEST()
