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
#include <unistd.h>

#include "../common/test.h"

#define UEVENT_PATH "/sys/devices/virtual/mem/null/uevent"
#define UUID "12345678-1234-1234-1234-123456789abc"
#define SYNTH_UUID "SYNTH_UUID=" UUID
#define CHANGE_ACTION "change " UUID

static int readonly_uevent_fd, writeonly_uevent_fd, seqnum_fd, uevent_socket;

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

static ssize_t receive_event(char *buffer, size_t capacity)
{
	for (;;) {
		struct sockaddr_nl source = { 0 };
		struct iovec iov = { .iov_base = buffer, .iov_len = capacity };
		struct msghdr message = {
			.msg_name = &source,
			.msg_namelen = sizeof(source),
			.msg_iov = &iov,
			.msg_iovlen = 1,
		};
		ssize_t len = recvmsg(uevent_socket, &message, 0);
		if (len < 0)
			return -1;
		if (!has_field(buffer, len, '\0', SYNTH_UUID))
			continue;

		if (message.msg_flags & MSG_TRUNC) {
			errno = EMSGSIZE;
			return -1;
		}
		if (message.msg_namelen != sizeof(source) ||
		    source.nl_family != AF_NETLINK || source.nl_pid != 0) {
			errno = EPROTO;
			return -1;
		}

		return len;
	}
}

FN_SETUP(prepare)
{
	readonly_uevent_fd = CHECK(open(UEVENT_PATH, O_RDONLY));
	writeonly_uevent_fd = CHECK(open(UEVENT_PATH, O_WRONLY));

	seqnum_fd = CHECK(open("/sys/kernel/uevent_seqnum", O_RDONLY));

	uevent_socket =
		CHECK(socket(AF_NETLINK, SOCK_DGRAM, NETLINK_KOBJECT_UEVENT));
	struct sockaddr_nl address = {
		.nl_family = AF_NETLINK,
		.nl_groups = 1,
	};
	CHECK(bind(uevent_socket, (struct sockaddr *)&address,
		   sizeof(address)));
}
END_SETUP()

FN_TEST(sysfs_environment_and_netlink_delivery)
{
	// 1. Read the device's environment through sysfs.
	char buffer[8192];
	TEST_RES(read(readonly_uevent_fd, buffer, sizeof(buffer)),
		 _ret > 0 && has_field(buffer, _ret, '\n', "DEVNAME=null"));

	// 2. Snapshot the sequence number and request a change event with the UUID.
	unsigned long long seqnum_before = read_seqnum();
	TEST_RES(write(writeonly_uevent_fd, CHANGE_ACTION,
		       sizeof(CHANGE_ACTION) - 1),
		 _ret == sizeof(CHANGE_ACTION) - 1);

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
	CHECK(close(uevent_socket));
	CHECK(close(seqnum_fd));
	CHECK(close(writeonly_uevent_fd));
	CHECK(close(readonly_uevent_fd));
}
END_SETUP()
