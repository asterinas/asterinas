/* SPDX-License-Identifier: MPL-2.0 */

#ifndef CAPABILITY_H
#define CAPABILITY_H

#include <linux/capability.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "test.h"

static inline int __read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	return syscall(SYS_capget, &cap_header, cap_data);
}

static inline void read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	CHECK(__read_cap_data(cap_data));
}

static inline int
__write_cap_data(const struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	return syscall(SYS_capset, &cap_header, cap_data);
}

static inline void
write_cap_data(const struct __user_cap_data_struct cap_data[2])
{
	CHECK(__write_cap_data(cap_data));
}

static inline void drop_capability(int capability)
{
	struct __user_cap_data_struct cap_data[2] = {};
	unsigned int cap_index = capability / 32;
	uint32_t cap_mask = 1U << (capability % 32);

	read_cap_data(cap_data);
	cap_data[cap_index].effective &= ~cap_mask;
	cap_data[cap_index].permitted &= ~cap_mask;
	cap_data[cap_index].inheritable &= ~cap_mask;
	write_cap_data(cap_data);
}

#endif /* CAPABILITY_H */
