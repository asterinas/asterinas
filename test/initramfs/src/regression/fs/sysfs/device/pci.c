// SPDX-License-Identifier: MPL-2.0

#include "../../../common/test.h"
#include "common.h"

static const struct device_attribute pci_attributes[] = {
	{ "vendor", 0444 },   { "device", 0444 }, { "class", 0444 },
	{ "revision", 0444 }, { NULL, 0 },
};

/* These paths are shared by the normal and TDX Q35 configurations in `tools/qemu_args.sh`. */
static const struct device_spec root_bus_devices[] = {
	/* PCI root bus 0 in segment 0, fixed by the Q35 chipset layout. */
	{
		.path = "/sys/devices/pci0000:00",
	},
	/* PCI host bridge at `00:00.0`, fixed by the Q35 chipset layout. */
	{
		.path = "/sys/devices/pci0000:00/0000:00:00.0",
		.subsystem = "/sys/bus/pci",
		.attributes = pci_attributes,
	},
	/* SMBus controller at `00:1f.3`, fixed by the Q35 chipset layout. */
	{
		.path = "/sys/devices/pci0000:00/0000:00:1f.3",
		.subsystem = "/sys/bus/pci",
		.attributes = pci_attributes,
	},
	/*
	 * VirtIO block device backed by `ext2.img` through `drive=x0`.
	 * `bus=pcie.0,addr=0x6` selects bus 0, device 6, with function 0 by default.
	 */
	{
		.path = "/sys/devices/pci0000:00/0000:00:06.0",
		.subsystem = "/sys/bus/pci",
		.driver = "/sys/bus/pci/drivers/virtio-pci",
		.attributes = pci_attributes,
	},
};

/* These devices are only configured by the normal Q35 configuration. */
static const struct device_spec normal_devices[] = {
	/* VGA controller, automatically assigned `00:01.0` in the default configuration. */
	{
		.path = "/sys/devices/pci0000:00/0000:00:01.0",
		.subsystem = "/sys/bus/pci",
		.attributes = pci_attributes,
	},
	/*
	 * NVMe controller backed by `nvme0n1.img` through `drive=nvme0n1`.
	 * `bus=pcie.0,addr=0x4` selects bus 0, device 4, with function 0 by default.
	 */
	{
		.path = "/sys/devices/pci0000:00/0000:00:04.0",
		.subsystem = "/sys/bus/pci",
		.driver = "/sys/bus/pci/drivers/nvme",
		.attributes = pci_attributes,
	},
};

FN_TEST(root_bus)
{
	for (size_t i = 0; i < ARRAY_SIZE(root_bus_devices); i++)
		TEST_SUCC(check_device(&root_bus_devices[i]));
}
END_TEST()

FN_TEST(normal_devices)
{
	/* TDX disables VGA and does not configure an NVMe controller. */
	const char *intel_tdx = getenv("INTEL_TDX");
	SKIP_TEST_IF(intel_tdx && !strcmp(intel_tdx, "1"));

	for (size_t i = 0; i < ARRAY_SIZE(normal_devices); i++)
		TEST_SUCC(check_device(&normal_devices[i]));
}
END_TEST()
