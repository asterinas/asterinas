// SPDX-License-Identifier: MPL-2.0

#include "../../../common/test.h"
#include "common.h"

static const struct device_attribute virtio_attributes[] = {
	{ "device", 0444 },
	{ "vendor", 0444 },
	{ "status", 0444 },
	{ NULL, 0 },
};

/* Fixed PCI addresses for `ext2.img`, `exfat.img`, and `ltp_dev.img` in both Q35 configurations. */
static const struct device_spec block_devices[] = {
	{
		.path = "/sys/devices/pci0000:00/0000:00:06.0/virtio[0-9]*",
		.subsystem = "/sys/bus/virtio",
		.driver = "/sys/bus/virtio/drivers/virtio_blk",
		.attributes = virtio_attributes,
	},
	{
		.path = "/sys/devices/pci0000:00/0000:00:07.0/virtio[0-9]*",
		.subsystem = "/sys/bus/virtio",
		.driver = "/sys/bus/virtio/drivers/virtio_blk",
		.attributes = virtio_attributes,
	},
	{
		.path = "/sys/devices/pci0000:00/0000:00:08.0/virtio[0-9]*",
		.subsystem = "/sys/bus/virtio",
		.driver = "/sys/bus/virtio/drivers/virtio_blk",
		.attributes = virtio_attributes,
	},
};

/* QEMU assigns the network and console functions after VGA at `00:01.0`. */
static const struct device_spec network_and_console_devices[] = {
	{
		.path = "/sys/devices/pci0000:00/0000:00:02.0/virtio[0-9]*",
		.subsystem = "/sys/bus/virtio",
		.driver = "/sys/bus/virtio/drivers/virtio_net",
		.attributes = virtio_attributes,
	},
	{
		.path = "/sys/devices/pci0000:00/0000:00:03.0/virtio[0-9]*",
		.subsystem = "/sys/bus/virtio",
		.driver = "/sys/bus/virtio/drivers/virtio_console",
		.attributes = virtio_attributes,
	},
};

FN_TEST(block_devices)
{
	for (size_t i = 0; i < ARRAY_SIZE(block_devices); i++)
		TEST_SUCC(check_device(&block_devices[i]));
}
END_TEST()

FN_TEST(network_and_console_devices)
{
	/* TDX uses a different PCI layout. These addresses follow the normal Q35 configuration. */
	const char *intel_tdx = getenv("INTEL_TDX");
	SKIP_TEST_IF(intel_tdx && !strcmp(intel_tdx, "1"));

	for (size_t i = 0; i < ARRAY_SIZE(network_and_console_devices); i++)
		TEST_SUCC(check_device(&network_and_console_devices[i]));
}
END_TEST()
