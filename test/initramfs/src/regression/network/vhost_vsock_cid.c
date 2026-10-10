// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <linux/vhost.h>
#include <linux/vm_sockets.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../common/test.h"

#define GUEST_CID 42

// Regression for the pre-owner CID allocation failure in PR #3808.
FN_TEST(cid_reservation_without_owner)
{
	int first = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	int second = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	uint64_t invalid[] = { VMADDR_CID_HYPERVISOR, VMADDR_CID_LOCAL,
			       VMADDR_CID_HOST, VMADDR_CID_ANY, UINT64_MAX };
	for (unsigned i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
		TEST_ERRNO(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &invalid[i]),
			   EINVAL);
	}
	TEST_ERRNO(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, NULL), EFAULT);

	uint64_t cid = GUEST_CID;
	TEST_SUCC(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &cid));
	TEST_SUCC(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &cid));
	TEST_ERRNO(ioctl(second, VHOST_VSOCK_SET_GUEST_CID, &cid), EADDRINUSE);

	int running = 1;
	TEST_ERRNO(ioctl(first, VHOST_VSOCK_SET_RUNNING, &running), EPERM);
	running = 0;
	TEST_ERRNO(ioctl(first, VHOST_VSOCK_SET_RUNNING, &running), EPERM);
	struct vhost_vring_state ring = { .index = 0, .num = 8 };
	TEST_ERRNO(ioctl(first, VHOST_SET_VRING_NUM, &ring), EPERM);
	struct vhost_memory memory = { .nregions = 0 };
	TEST_ERRNO(ioctl(first, VHOST_SET_MEM_TABLE, &memory), EPERM);

	uint64_t other_cid = GUEST_CID + 1;
	TEST_SUCC(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &other_cid));
	TEST_SUCC(ioctl(second, VHOST_VSOCK_SET_GUEST_CID, &cid));
	CHECK(close(first));
	TEST_SUCC(ioctl(second, VHOST_VSOCK_SET_GUEST_CID, &other_cid));
	CHECK(close(second));
}
END_TEST()

FN_TEST(cid_fd_handoff)
{
	int fd = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	uint64_t cid = GUEST_CID;
	TEST_SUCC(ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, &cid));
	struct vhost_vring_state ring = { .index = 0, .num = 8 };

	pid_t child = CHECK(fork());
	if (child == 0) {
		CHECK(ioctl(fd, VHOST_SET_OWNER));
		CHECK(ioctl(fd, VHOST_SET_VRING_NUM, &ring));
		CHECK(ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, &cid));
		CHECK(close(fd));
		_exit(0);
	}
	int status;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);

	// The reservation survives the child's close, but queue ownership is its own.
	int other = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	TEST_ERRNO(ioctl(other, VHOST_VSOCK_SET_GUEST_CID, &cid), EADDRINUSE);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_NUM, &ring), EPERM);
	TEST_SUCC(ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, &cid));
	CHECK(close(fd));
	TEST_SUCC(ioctl(other, VHOST_VSOCK_SET_GUEST_CID, &cid));
	CHECK(close(other));
}
END_TEST()
