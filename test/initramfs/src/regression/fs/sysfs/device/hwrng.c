// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <unistd.h>

#include "../../../common/test.h"
#include "common.h"

#define HWRNG_SYSFS "/sys/class/misc/hw_random/"

static const struct device_attribute hwrng_attributes[] = {
	{ "dev", 0444 },
	{ "rng_available", 0444 },
	{ "rng_current", 0644 },
	{ NULL, 0 },
};

static const struct device_node hwrng_node = {
	.path = "/dev/hwrng",
	.mode = S_IFCHR | 0600,
};

static const struct device_spec hw_random = {
	.path = "/sys/devices/virtual/misc/hw_random",
	.subsystem = "/sys/class/misc",
	.attributes = hwrng_attributes,
	.node = &hwrng_node,
};

FN_TEST(hw_random_device)
{
	TEST_SUCC(check_device(&hw_random));
}
END_TEST()

static int attribute_equals(int fd, const char *expected)
{
	char buf[64];
	ssize_t n = read(fd, buf, sizeof(buf));

	return n == (ssize_t)strlen(expected) && !memcmp(buf, expected, n);
}

FN_TEST(entropy_source_attributes)
{
	const char *intel_tdx = getenv("INTEL_TDX");
	int available = TEST_SUCC(open(HWRNG_SYSFS "rng_available", O_RDONLY));
	int current = TEST_SUCC(open(HWRNG_SYSFS "rng_current", O_RDONLY));

	/* The default Q35 guest has one VirtIO RNG. The TDX guest has none. */
	if (intel_tdx && !strcmp(intel_tdx, "1")) {
		TEST_RES(attribute_equals(available, "\n"), _ret);
		TEST_RES(attribute_equals(current, "none\n"), _ret);
	} else {
		TEST_RES(attribute_equals(available, "virtio_rng.0 \n"), _ret);
		TEST_RES(attribute_equals(current, "virtio_rng.0\n"), _ret);
	}

	TEST_SUCC(close(current));
	TEST_SUCC(close(available));
}
END_TEST()
