// SPDX-License-Identifier: MPL-2.0

#include <libgen.h>

#include "../../../common/test.h"
#include "common.h"

static const struct device_attribute disk_attributes[] = {
	{ "dev", 0444 },
	{ "size", 0444 },
	{ NULL, 0 },
};

static int check_disk(const char *disk_pattern)
{
	/* Resolve the disk's canonical path through `/sys/class/block/<name>`. */
	const char *name = strrchr(disk_pattern, '/') + 1;
	char class_link[PATH_MAX], disk_path[PATH_MAX];
	snprintf(class_link, sizeof(class_link), "/sys/class/block/%s", name);
	if (!realpath(class_link, disk_path))
		return device_error(class_link, "realpath failed: %s",
				    strerror(errno));

	/* The whole-disk link in `/sys/block` must point to the same device. */
	char block_link[PATH_MAX];
	snprintf(block_link, sizeof(block_link), "/sys/block/%s", name);
	if (check_link(block_link, disk_path) < 0)
		return -1;

	/* Find the expected `device` link target, skipping VirtIO's `block/` directory. */
	char *parent = dirname(disk_path);
	if (!strcmp(strrchr(parent, '/') + 1, "block"))
		parent = dirname(parent);

	/* Expect a block device node at `/dev/<name>` with mode 0600. */
	char node_path[PATH_MAX];
	snprintf(node_path, sizeof(node_path), "/dev/%s", name);
	const struct device_node disk_node = { .path = node_path,
					       .mode = S_IFBLK | 0600 };

	/* Check the expected sysfs location, class and parent links, attributes, and node. */
	const struct device_spec spec = {
		.path = disk_pattern,
		.subsystem = "/sys/class/block",
		.device_link = parent,
		.attributes = disk_attributes,
		.node = &disk_node,
	};
	return check_device(&spec);
}

FN_TEST(virtio_disks)
{
	TEST_SUCC(check_disk(
		"/sys/devices/pci0000:00/0000:00:06.0/virtio[0-9]*/block/vda"));
	TEST_SUCC(check_disk(
		"/sys/devices/pci0000:00/0000:00:07.0/virtio[0-9]*/block/vdb"));
	TEST_SUCC(check_disk(
		"/sys/devices/pci0000:00/0000:00:08.0/virtio[0-9]*/block/vdc"));
}
END_TEST()

FN_TEST(nvme_namespace)
{
	/* The TDX Q35 configuration has no NVMe controller. */
	const char *intel_tdx = getenv("INTEL_TDX");
	SKIP_TEST_IF(intel_tdx && !strcmp(intel_tdx, "1"));

	TEST_SUCC(check_disk(
		"/sys/devices/pci0000:00/0000:00:04.0/nvme/nvme0/nvme0n1"));
}
END_TEST()
