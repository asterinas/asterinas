// SPDX-License-Identifier: MPL-2.0

#include "../../../common/test.h"
#include "common.h"

static const struct device_attribute mem_attributes[] = {
	{ "dev", 0444 },
	{ NULL, 0 },
};

static const struct device_node full_node = {
	.path = "/dev/full",
	.mode = S_IFCHR | 0666,
};

static const struct device_node null_node = {
	.path = "/dev/null",
	.mode = S_IFCHR | 0666,
};

static const struct device_node random_node = {
	.path = "/dev/random",
	.mode = S_IFCHR | 0666,
};

static const struct device_node urandom_node = {
	.path = "/dev/urandom",
	.mode = S_IFCHR | 0666,
};

static const struct device_node zero_node = {
	.path = "/dev/zero",
	.mode = S_IFCHR | 0666,
};

static const struct device_spec mem_devices[] = {
	{
		.path = "/sys/devices/virtual/mem/full",
		.subsystem = "/sys/class/mem",
		.attributes = mem_attributes,
		.node = &full_node,
	},
	{
		.path = "/sys/devices/virtual/mem/null",
		.subsystem = "/sys/class/mem",
		.attributes = mem_attributes,
		.node = &null_node,
	},
	{
		.path = "/sys/devices/virtual/mem/random",
		.subsystem = "/sys/class/mem",
		.attributes = mem_attributes,
		.node = &random_node,
	},
	{
		.path = "/sys/devices/virtual/mem/urandom",
		.subsystem = "/sys/class/mem",
		.attributes = mem_attributes,
		.node = &urandom_node,
	},
	{
		.path = "/sys/devices/virtual/mem/zero",
		.subsystem = "/sys/class/mem",
		.attributes = mem_attributes,
		.node = &zero_node,
	},
};

FN_TEST(mem_devices)
{
	for (size_t i = 0; i < ARRAY_SIZE(mem_devices); i++)
		TEST_SUCC(check_device(&mem_devices[i]));
}
END_TEST()
