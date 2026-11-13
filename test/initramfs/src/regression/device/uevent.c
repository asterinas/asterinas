// SPDX-License-Identifier: MPL-2.0

/*
 * Verify the device's sysfs environment and synthetic event delivery via Netlink.
 * Requires writable sysfs and permission to emit events.
 * Reference: <https://elixir.bootlin.com/linux/v6.14/source/Documentation/ABI/testing/sysfs-uevent>
 */

#include <fcntl.h>
#include <linux/netlink.h>
#include <stdbool.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#include "../common/test.h"

#define UEVENT_PATH "/sys/devices/virtual/mem/null/uevent"

static int environment_fd, request_fd, listener_fd, seqnum_fd;
static char request[64], uuid_field[64];
static size_t request_len;

static bool has_field(const char *buffer, size_t len, char delimiter,
		      const char *field)
{
	size_t field_len = strlen(field);
	while (len) {
		const char *end = memchr(buffer, delimiter, len);
		if (!end)
			return false;
		size_t entry_len = end - buffer;
		if (entry_len == field_len && !memcmp(buffer, field, field_len))
			return true;
		buffer = end + 1;
		len -= entry_len + 1;
	}
	return false;
}

static unsigned long long read_seqnum(void)
{
	char buffer[32];
	ssize_t len = pread(seqnum_fd, buffer, sizeof(buffer) - 1, 0);
	if (len <= 0)
		return 0;
	buffer[len] = '\0';
	return strtoull(buffer, NULL, 10);
}

static ssize_t read_environment(char *buffer, size_t capacity)
{
	size_t used = 0;
	while (used < capacity) {
		ssize_t len =
			read(environment_fd, buffer + used, capacity - used);
		if (len < 0)
			return -1;
		if (!len)
			return used;
		used += len;
	}
	errno = EOVERFLOW;
	return -1;
}

static ssize_t receive_event(char *buffer, size_t capacity)
{
	struct timespec deadline;
	if (clock_gettime(CLOCK_MONOTONIC, &deadline))
		return -1;
	/* Bound scanning of unrelated events, not waiting for our own event. */
	deadline.tv_sec++;
	for (;;) {
		struct sockaddr_nl source = { 0 };
		struct iovec iov = { .iov_base = buffer, .iov_len = capacity };
		struct msghdr message = {
			.msg_name = &source,
			.msg_namelen = sizeof(source),
			.msg_iov = &iov,
			.msg_iovlen = 1,
		};
		ssize_t len = recvmsg(listener_fd, &message, MSG_DONTWAIT);
		if (len < 0)
			return -1;
		if (has_field(buffer, len, '\0', uuid_field)) {
			if (message.msg_flags & MSG_TRUNC) {
				errno = EMSGSIZE;
				return -1;
			}
			if (message.msg_namelen != sizeof(source) ||
			    source.nl_family != AF_NETLINK ||
			    source.nl_pid != 0) {
				errno = EPROTO;
				return -1;
			}
			return len;
		}
		struct timespec now;
		if (clock_gettime(CLOCK_MONOTONIC, &now))
			return -1;
		if (now.tv_sec > deadline.tv_sec ||
		    (now.tv_sec == deadline.tv_sec &&
		     now.tv_nsec >= deadline.tv_nsec)) {
			errno = ETIMEDOUT;
			return -1;
		}
	}
}

FN_SETUP(prepare)
{
	environment_fd = CHECK(open(UEVENT_PATH, O_RDONLY));
	request_fd = CHECK(open(UEVENT_PATH, O_WRONLY));
	seqnum_fd = CHECK(open("/sys/kernel/uevent_seqnum", O_RDONLY));
	listener_fd = CHECK(socket(AF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
				   NETLINK_KOBJECT_UEVENT));
	struct sockaddr_nl address = {
		.nl_family = AF_NETLINK,
		.nl_groups = 1,
	};
	CHECK(bind(listener_fd, (struct sockaddr *)&address, sizeof(address)));

	struct timespec now;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &now));
	char uuid[37];
	CHECK_WITH(snprintf(uuid, sizeof(uuid), "%08x-%04x-%04x-0000-%012llx",
			    (unsigned)getpid(), (unsigned)now.tv_nsec >> 16,
			    (unsigned)now.tv_nsec & 0xffff,
			    (unsigned long long)now.tv_sec & 0xffffffffffffULL),
		   _ret == sizeof(uuid) - 1);
	CHECK_WITH(snprintf(uuid_field, sizeof(uuid_field), "SYNTH_UUID=%s",
			    uuid),
		   _ret >= 0 && _ret < sizeof(uuid_field));
	request_len = CHECK_WITH(snprintf(request, sizeof(request),
					  "change %s\n", uuid),
				 _ret >= 0 && _ret < sizeof(request));
}
END_SETUP()

FN_TEST(sysfs_environment_and_netlink_delivery)
{
	// 1. Read the device's environment through sysfs.
	char buffer[8192];
	TEST_RES(read_environment(buffer, sizeof(buffer)),
		 _ret > 0 && has_field(buffer, _ret, '\n', "DEVNAME=null"));

	// 2. Snapshot the sequence number and request a change event carrying
	//    this run's UUID.
	unsigned long long seqnum_before = read_seqnum();
	TEST_RES(write(request_fd, request, request_len), _ret == request_len);

	// 3. The counter advances at least once after the write.
	TEST_RES(read_seqnum(), _ret > seqnum_before);

	// 4. The synchronous write has already queued the target event.
	static const char prefix[] = "change@/devices/virtual/mem/null";
	TEST_RES(receive_event(buffer, sizeof(buffer)),
		 _ret >= (ssize_t)sizeof(prefix) &&
			 memcmp(buffer, prefix, sizeof(prefix)) == 0 &&
			 has_field(buffer, _ret, '\0', "ACTION=change") &&
			 has_field(buffer, _ret, '\0',
				   "DEVPATH=/devices/virtual/mem/null") &&
			 has_field(buffer, _ret, '\0', "SUBSYSTEM=mem"));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(listener_fd));
	CHECK(close(request_fd));
	CHECK(close(environment_fd));
	CHECK(close(seqnum_fd));
}
END_SETUP()
