// SPDX-License-Identifier: MPL-2.0

#include <linux/perf_event.h>
#include <sys/ioctl.h>

#include "../../common/capability.h"

static uint64_t read_count(int fd)
{
	uint64_t count;

	CHECK_WITH(read(fd, &count, sizeof(count)), _ret == sizeof(count));
	return count;
}

FN_TEST(effective_capability_gain_revokes_perf_event)
{
	const uint32_t capability = 1U << CAP_NET_BIND_SERVICE;
	struct __user_cap_data_struct caps[2];
	struct perf_event_attr attr = {
		.type = PERF_TYPE_HARDWARE,
		.size = sizeof(attr),
		.config = PERF_COUNT_HW_INSTRUCTIONS,
		.exclude_kernel = 1,
	};

	read_cap_data(caps);
	CHECK_WITH(caps[0].effective & capability, _ret != 0);

	int fd = CHECK(syscall(SYS_perf_event_open, &attr, 0, -1, -1, 0UL));
	uint64_t before = read_count(fd);

	write_cap_data(caps);
	uint64_t after_noop = read_count(fd);
	TEST_RES(after_noop, _ret > before);

	caps[0].effective &= ~capability;
	write_cap_data(caps);
	uint64_t after_reduction = read_count(fd);
	TEST_RES(after_reduction, _ret > after_noop);

	// An effective capability gain freezes events opened under the old credentials.
	caps[0].effective |= capability;
	write_cap_data(caps);
	uint64_t frozen = read_count(fd);
	TEST_RES(frozen, _ret >= after_reduction);
	TEST_ERRNO(ioctl(fd, PERF_EVENT_IOC_ENABLE, 0), EACCES);
	TEST_RES(read_count(fd), _ret == frozen);

	TEST_SUCC(close(fd));
}
END_TEST()
