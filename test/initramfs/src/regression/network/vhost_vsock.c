// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <linux/vhost.h>
#include <linux/virtio_config.h>
#include <linux/virtio_ring.h>
#include <linux/virtio_vsock.h>
#include <poll.h>
#include <limits.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <linux/vm_sockets.h>

#include "../common/test.h"

#define GUEST_CID 42
#define PEER_PORT 4000
#define RING_SIZE 8
#define RX_HEAD 0
#define RX_HEADER_DESC 0
#define RX_PAYLOAD_DESC 1
#define TX_HEAD 2
#define TX_HEADER_DESC 2
#define TX_PAYLOAD_DESC 3
#define RX_HEADER_LEN 44
#define RX_PAYLOAD_LEN 128
#define TX_PAYLOAD_LEN 128
#define PAGE_SIZE 4096
#define ALIGN_UP(value, align) (((value) + (align) - 1) & ~((align) - 1))
#define INDIRECT_TABLE_LEN ((RING_SIZE + 1) * sizeof(struct vring_desc))

struct vhost_vsock_ring {
	struct vring_desc desc[RING_SIZE];
	struct {
		uint16_t flags;
		uint16_t idx;
		uint16_t ring[RING_SIZE];
	} avail;
	struct {
		uint16_t flags;
		uint16_t idx;
		struct vring_used_elem ring[RING_SIZE];
	} used;
};

struct vhost_vsock_fixture {
	struct {
		struct vhost_memory memory;
		struct vhost_memory_region region;
	} mem;
	struct vhost_vsock_ring rx;
	struct vhost_vsock_ring tx;
	struct virtio_vsock_hdr rx_header;
	uint8_t rx_payload[RX_PAYLOAD_LEN];
	struct virtio_vsock_hdr tx_header;
	uint8_t tx_payload[TX_PAYLOAD_LEN];
	struct vring_desc indirect_desc[RING_SIZE + 1];
	int fd;
	int rx_kick;
	int rx_call;
	int tx_kick;
	int tx_call;
};

static int vhost_fd;
static struct vhost_vsock_fixture fixture __attribute__((aligned(PAGE_SIZE)));
static struct vhost_vsock_fixture second_fixture
	__attribute__((aligned(PAGE_SIZE)));
static uint8_t mem_table_page[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));
static uint8_t split_mem[2 * PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));

static int open_vhost_vsock(void)
{
	return open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC);
}

FN_SETUP(exec_owner_child)
{
	const char *fd_env = getenv("VHOST_VSOCK_EXEC_OWNER_FD");
	char *endptr;
	long fd_long;
	uint64_t features = 0;
	int set_ret;
	int set_errno;
	int reset_ret;
	int reset_errno;

	if (fd_env == NULL)
		return;

	fd_long = strtol(fd_env, &endptr, 10);
	if (*fd_env == '\0' || *endptr != '\0' || fd_long < 0 ||
	    fd_long > INT_MAX)
		_exit(2);

	set_ret = ioctl((int)fd_long, VHOST_SET_FEATURES, &features);
	set_errno = errno;
	reset_ret = ioctl((int)fd_long, VHOST_RESET_OWNER);
	reset_errno = errno;

	if (set_ret < 0 && set_errno == EPERM && reset_ret < 0 &&
	    reset_errno == EPERM)
		_exit(0);

	_exit(1);
}
END_SETUP()

static void close_fd_if_open(int *fd)
{
	if (*fd >= 0) {
		CHECK(close(*fd));
		*fd = -1;
	}
}

static void reset_vhost_fixture(struct vhost_vsock_fixture *f)
{
	memset(f, 0, sizeof(*f));
	f->fd = -1;
	f->rx_kick = -1;
	f->rx_call = -1;
	f->tx_kick = -1;
	f->tx_call = -1;
}

static void reset_fixture(void)
{
	reset_vhost_fixture(&fixture);
}

static int wait_eventfd(int fd)
{
	struct pollfd pfd = {
		.fd = fd,
		.events = POLLIN,
	};

	if (poll(&pfd, 1, 3000) <= 0) {
		errno = ETIMEDOUT;
		return -1;
	}

	uint64_t counter = 0;
	return read(fd, &counter, sizeof(counter)) == sizeof(counter) ? 0 : -1;
}

static int kick_eventfd(int fd)
{
	uint64_t counter = 1;
	return write(fd, &counter, sizeof(counter)) == sizeof(counter) ? 0 : -1;
}

static void setup_rx_buffer_chain(struct vhost_vsock_fixture *f)
{
	f->rx.desc[RX_HEADER_DESC].addr = (uintptr_t)&f->rx_header;
	f->rx.desc[RX_HEADER_DESC].len = RX_HEADER_LEN;
	f->rx.desc[RX_HEADER_DESC].flags = VRING_DESC_F_NEXT |
					   VRING_DESC_F_WRITE;
	f->rx.desc[RX_HEADER_DESC].next = RX_PAYLOAD_DESC;
	f->rx.desc[RX_PAYLOAD_DESC].addr = (uintptr_t)f->rx_payload;
	f->rx.desc[RX_PAYLOAD_DESC].len = sizeof(f->rx_payload);
	f->rx.desc[RX_PAYLOAD_DESC].flags = VRING_DESC_F_WRITE;
	f->rx.avail.ring[f->rx.avail.idx % RING_SIZE] = RX_HEAD;
	f->rx.avail.idx++;
}

static void setup_tx_packet(struct vhost_vsock_fixture *f, uint64_t src_cid,
			    uint32_t src_port, uint32_t dst_port, uint16_t op,
			    const void *payload, uint32_t payload_len)
{
	memset(&f->tx_header, 0, sizeof(f->tx_header));
	memset(f->tx_payload, 0, sizeof(f->tx_payload));

	f->tx_header.src_cid = src_cid;
	f->tx_header.dst_cid = VMADDR_CID_HOST;
	f->tx_header.src_port = src_port;
	f->tx_header.dst_port = dst_port;
	f->tx_header.len = payload_len;
	f->tx_header.type = VIRTIO_VSOCK_TYPE_STREAM;
	f->tx_header.op = op;
	f->tx_header.buf_alloc = 256 * 1024;

	if (payload_len != 0)
		memcpy(f->tx_payload, payload, payload_len);

	f->tx.desc[TX_HEADER_DESC].addr = (uintptr_t)&f->tx_header;
	f->tx.desc[TX_HEADER_DESC].len = sizeof(f->tx_header);
	f->tx.desc[TX_HEADER_DESC].flags =
		payload_len != 0 ? VRING_DESC_F_NEXT : 0;
	f->tx.desc[TX_HEADER_DESC].next = TX_PAYLOAD_DESC;
	f->tx.desc[TX_PAYLOAD_DESC].addr = (uintptr_t)f->tx_payload;
	f->tx.desc[TX_PAYLOAD_DESC].len = payload_len;
	f->tx.desc[TX_PAYLOAD_DESC].flags = 0;
	f->tx.avail.ring[f->tx.avail.idx % RING_SIZE] = TX_HEAD;
	f->tx.avail.idx++;
}

static void configure_vhost_device_without_running_with_eventfd_flags(
	struct vhost_vsock_fixture *f, uint64_t guest_cid, int eventfd_flags)
{
	uint64_t features = 0;
	struct vhost_vring_state state = { 0 };
	struct vhost_vring_addr addr = { 0 };
	struct vhost_vring_file file = { 0 };

	f->fd = CHECK(open_vhost_vsock());
	f->rx_kick = CHECK(eventfd(0, eventfd_flags));
	f->rx_call = CHECK(eventfd(0, eventfd_flags));
	f->tx_kick = CHECK(eventfd(0, eventfd_flags));
	f->tx_call = CHECK(eventfd(0, eventfd_flags));

	f->mem.memory.nregions = 1;
	f->mem.region.guest_phys_addr = (uintptr_t)f;
	f->mem.region.memory_size = ALIGN_UP(sizeof(*f), PAGE_SIZE);
	f->mem.region.userspace_addr = (uintptr_t)f;

	CHECK(ioctl(f->fd, VHOST_SET_OWNER));
	CHECK(ioctl(f->fd, VHOST_GET_FEATURES, &features));
	CHECK(ioctl(f->fd, VHOST_SET_FEATURES, &features));
	features = 0;
	CHECK(ioctl(f->fd, VHOST_SET_BACKEND_FEATURES, &features));
	CHECK(ioctl(f->fd, VHOST_SET_MEM_TABLE, &f->mem.memory));

	state.num = RING_SIZE;
	state.index = 0;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_NUM, &state));
	state.index = 1;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_NUM, &state));

	addr.index = 0;
	addr.desc_user_addr = (uintptr_t)f->rx.desc;
	addr.avail_user_addr = (uintptr_t)&f->rx.avail;
	addr.used_user_addr = (uintptr_t)&f->rx.used;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_ADDR, &addr));
	addr.index = 1;
	addr.desc_user_addr = (uintptr_t)f->tx.desc;
	addr.avail_user_addr = (uintptr_t)&f->tx.avail;
	addr.used_user_addr = (uintptr_t)&f->tx.used;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_ADDR, &addr));

	file.index = 0;
	file.fd = f->rx_kick;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_KICK, &file));
	file.fd = f->rx_call;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_CALL, &file));
	file.index = 1;
	file.fd = f->tx_kick;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_KICK, &file));
	file.fd = f->tx_call;
	CHECK(ioctl(f->fd, VHOST_SET_VRING_CALL, &file));

	CHECK(ioctl(f->fd, VHOST_VSOCK_SET_GUEST_CID, &guest_cid));
}

static void
configure_vhost_device_without_running(struct vhost_vsock_fixture *f,
				       uint64_t guest_cid)
{
	configure_vhost_device_without_running_with_eventfd_flags(
		f, guest_cid, EFD_CLOEXEC | EFD_NONBLOCK);
}

static void start_vhost_device(struct vhost_vsock_fixture *f)
{
	int running = 1;

	CHECK(ioctl(f->fd, VHOST_VSOCK_SET_RUNNING, &running));
}

static void configure_vhost_device(struct vhost_vsock_fixture *f,
				   uint64_t guest_cid)
{
	configure_vhost_device_without_running(f, guest_cid);
	start_vhost_device(f);
}

static int teardown_vhost_device(struct vhost_vsock_fixture *f)
{
	int running = 0;

	if (f->fd >= 0) {
		if (ioctl(f->fd, VHOST_VSOCK_SET_RUNNING, &running) < 0)
			return -1;
		if (ioctl(f->fd, VHOST_RESET_OWNER) < 0)
			return -1;
	}

	close_fd_if_open(&f->fd);
	close_fd_if_open(&f->rx_kick);
	close_fd_if_open(&f->rx_call);
	close_fd_if_open(&f->tx_kick);
	close_fd_if_open(&f->tx_call);
	return 0;
}

static void close_vhost_eventfds(struct vhost_vsock_fixture *f)
{
	close_fd_if_open(&f->rx_kick);
	close_fd_if_open(&f->rx_call);
	close_fd_if_open(&f->tx_kick);
	close_fd_if_open(&f->tx_call);
}

static int connect_to_guest(uint32_t port)
{
	int fd = socket(AF_VSOCK, SOCK_STREAM | SOCK_NONBLOCK, 0);
	struct sockaddr_vm addr = {
		.svm_family = AF_VSOCK,
		.svm_cid = GUEST_CID,
		.svm_port = port,
	};

	if (fd < 0)
		return -1;
	if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) == 0) {
		errno = 0;
		return fd;
	}
	if (errno == EINPROGRESS) {
		errno = 0;
		return fd;
	}

	close(fd);
	return -1;
}

static int wait_socket_writable(int fd)
{
	struct pollfd pfd = {
		.fd = fd,
		.events = POLLOUT,
	};
	socklen_t error_len;
	int error = 0;

	if (poll(&pfd, 1, 3000) <= 0) {
		errno = ETIMEDOUT;
		return -1;
	}
	if ((pfd.revents & POLLOUT) == 0) {
		errno = EPROTO;
		return -1;
	}
	error_len = sizeof(error);
	if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &error_len) < 0)
		return -1;
	if (error != 0) {
		errno = error;
		return -1;
	}

	return 0;
}

static int wait_worker_stopped(int fd)
{
	int running = 1;

	for (int attempt = 0; attempt < 100; attempt++) {
		if (ioctl(fd, VHOST_VSOCK_SET_RUNNING, &running) < 0) {
			if (errno == EIO) {
				errno = 0;
				return 0;
			}
			return -1;
		}
		usleep(10000);
	}

	errno = ETIMEDOUT;
	return -1;
}

static int expect_rx_packet(struct vhost_vsock_fixture *f, uint16_t op,
			    uint32_t dst_port, const char *payload)
{
	if (wait_eventfd(f->rx_call) < 0)
		return -1;
	if (f->rx.used.idx != f->rx.avail.idx ||
	    f->rx_header.src_cid != VMADDR_CID_HOST ||
	    f->rx_header.dst_cid != GUEST_CID ||
	    f->rx_header.dst_port != dst_port || f->rx_header.op != op) {
		errno = EPROTO;
		return -1;
	}

	if (payload != NULL) {
		size_t payload_len = strlen(payload);

		if (f->rx_header.len != payload_len ||
		    memcmp(f->rx_payload, payload, payload_len) != 0) {
			errno = EPROTO;
			return -1;
		}
	}

	return 0;
}

FN_SETUP(open_device)
{
	reset_fixture();
	vhost_fd = CHECK(open_vhost_vsock());
}
END_SETUP()

FN_TEST(features)
{
	uint64_t features = 0;
	uint64_t unsupported = 1ULL << 63;

	TEST_SUCC(ioctl(vhost_fd, VHOST_GET_FEATURES, &features));
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_FEATURES, &features), EPERM);

	TEST_SUCC(ioctl(vhost_fd, VHOST_SET_OWNER));
	TEST_RES(ioctl(vhost_fd, VHOST_SET_FEATURES, &features), _ret == 0);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_FEATURES, &unsupported), EINVAL);

	TEST_SUCC(ioctl(vhost_fd, VHOST_GET_BACKEND_FEATURES, &features));
	TEST_RES(features, features == 0);
	TEST_RES(ioctl(vhost_fd, VHOST_SET_BACKEND_FEATURES, &features),
		 _ret == 0);
	unsupported = 1;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_BACKEND_FEATURES, &unsupported),
		   EINVAL);
	TEST_SUCC(ioctl(vhost_fd, VHOST_RESET_OWNER));
}
END_TEST()

FN_TEST(owner_required_for_setup_ioctls)
{
	int fd = TEST_SUCC(open_vhost_vsock());
	uint64_t features = 0;
	uint64_t guest_cid = GUEST_CID;
	int running = 1;
	struct vhost_memory memory = { 0 };
	struct vhost_vring_state state = {
		.index = 0,
		.num = RING_SIZE,
	};
	struct vhost_vring_addr addr = {
		.index = 0,
		.desc_user_addr = (uintptr_t)fixture.rx.desc,
		.avail_user_addr = (uintptr_t)&fixture.rx.avail,
		.used_user_addr = (uintptr_t)&fixture.rx.used,
	};
	struct vhost_vring_file file = {
		.index = 0,
		.fd = -1,
	};

	TEST_ERRNO(ioctl(fd, VHOST_SET_FEATURES, &features), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_BACKEND_FEATURES, &features), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_MEM_TABLE, &memory), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_NUM, &state), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_BASE, &state), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_GET_VRING_BASE, &state), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_ADDR, &addr), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_KICK, &file), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_CALL, &file), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_SET_VRING_ERR, &file), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, &guest_cid), EPERM);
	TEST_ERRNO(ioctl(fd, VHOST_VSOCK_SET_RUNNING, &running), EPERM);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(owner_lifecycle)
{
	TEST_SUCC(ioctl(vhost_fd, VHOST_SET_OWNER));
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_OWNER), EBUSY);
	TEST_SUCC(ioctl(vhost_fd, VHOST_RESET_OWNER));
	TEST_SUCC(ioctl(vhost_fd, VHOST_SET_OWNER));
}
END_TEST()

FN_TEST(owner_process_required)
{
	int fd = TEST_SUCC(open_vhost_vsock());
	uint64_t features = 0;
	int status = 0;
	pid_t child;

	TEST_SUCC(ioctl(fd, VHOST_SET_OWNER));
	child = TEST_SUCC(fork());
	if (child == 0) {
		if (ioctl(fd, VHOST_SET_FEATURES, &features) < 0 &&
		    errno == EPERM && ioctl(fd, VHOST_RESET_OWNER) < 0 &&
		    errno == EPERM) {
			_exit(0);
		}
		_exit(1);
	}

	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(ioctl(fd, VHOST_RESET_OWNER));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(owner_address_space_required_after_exec)
{
	int fd = TEST_SUCC(open_vhost_vsock());
	char fd_env[64];
	char self_path[PATH_MAX];
	ssize_t len;
	int flags;
	int status = 0;
	pid_t child;

	flags = TEST_SUCC(fcntl(fd, F_GETFD));
	TEST_SUCC(fcntl(fd, F_SETFD, flags & ~FD_CLOEXEC));
	TEST_SUCC(ioctl(fd, VHOST_SET_OWNER));

	len = TEST_SUCC(
		readlink("/proc/self/exe", self_path, sizeof(self_path) - 1));
	self_path[len] = '\0';
	TEST_RES(snprintf(fd_env, sizeof(fd_env),
			  "VHOST_VSOCK_EXEC_OWNER_FD=%d", fd),
		 _ret > 0 && (size_t)_ret < sizeof(fd_env));

	child = TEST_SUCC(fork());
	if (child == 0) {
		char *const argv[] = { self_path, NULL };
		char *const envp[] = { fd_env, NULL };

		execve(self_path, argv, envp);
		_exit(127);
	}

	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(ioctl(fd, VHOST_RESET_OWNER));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(guest_cid_validation)
{
	uint64_t reserved_cid = 2;
	uint64_t guest_cid = 3;
	uint64_t max_u32_cid = UINT32_MAX;
	uint64_t over_u32_cid = (uint64_t)UINT32_MAX + 1;

	TEST_ERRNO(ioctl(vhost_fd, VHOST_VSOCK_SET_GUEST_CID, &reserved_cid),
		   EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_VSOCK_SET_GUEST_CID, &max_u32_cid),
		   EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_VSOCK_SET_GUEST_CID, &over_u32_cid),
		   EINVAL);
	TEST_SUCC(ioctl(vhost_fd, VHOST_VSOCK_SET_GUEST_CID, &guest_cid));
}
END_TEST()

FN_TEST(vring_index_validation)
{
	int regular_fd;
	struct vhost_vring_state state = {
		.index = 2,
		.num = 8,
	};
	struct vhost_vring_addr addr = {
		.index = 0,
		.desc_user_addr = UINTPTR_MAX,
		.avail_user_addr = (uintptr_t)&fixture.rx.avail,
		.used_user_addr = (uintptr_t)&fixture.rx.used,
	};
	struct vhost_vring_file file = {
		.index = 2,
		.fd = -1,
	};

	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_NUM, &state), EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_BASE, &state), EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_GET_VRING_BASE, &state), EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_KICK, &file), EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_CALL, &file), EINVAL);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_ERR, &file), EINVAL);

	state.index = 0;
	state.num = 0;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_NUM, &state), EINVAL);
	state.num = 7;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_NUM, &state), EINVAL);
	state.num = 8;
	TEST_SUCC(ioctl(vhost_fd, VHOST_SET_VRING_NUM, &state));
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_ADDR, &addr), EINVAL);

	addr.desc_user_addr = 0;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_ADDR, &addr), EINVAL);

	addr.desc_user_addr = (uintptr_t)fixture.rx.desc;
	addr.flags = 1;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_ADDR, &addr), EINVAL);
	addr.flags = 0;
	addr.log_guest_addr = (uintptr_t)mem_table_page;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_ADDR, &addr), EINVAL);

	state.num = (uint32_t)UINT16_MAX + 1;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_BASE, &state), EINVAL);

	regular_fd = TEST_SUCC(open("/dev/null", O_RDONLY | O_CLOEXEC));
	file.index = 0;
	file.fd = regular_fd;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_VRING_KICK, &file), EINVAL);
	TEST_SUCC(close(regular_fd));
}
END_TEST()

FN_TEST(mem_table_validation)
{
	struct {
		struct vhost_memory memory;
		struct vhost_memory_region region;
	} table = { 0 };

	table.memory.padding = 1;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory), EINVAL);

	table.memory.padding = 0;
	table.memory.nregions = 65;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory), EINVAL);

	table.memory.nregions = 1;
	table.region.flags_padding = 1;
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory), EINVAL);

	table.region.flags_padding = 0;
	table.region.guest_phys_addr = (uintptr_t)mem_table_page + 1;
	table.region.userspace_addr = (uintptr_t)mem_table_page;
	table.region.memory_size = sizeof(mem_table_page);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory), EINVAL);

	table.region.guest_phys_addr = (uintptr_t)mem_table_page;
	table.region.userspace_addr = 0;
	table.region.memory_size = sizeof(mem_table_page);
	TEST_ERRNO(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory), EINVAL);

	table.region.guest_phys_addr = (uintptr_t)mem_table_page;
	table.region.userspace_addr = (uintptr_t)mem_table_page;
	table.region.memory_size = sizeof(mem_table_page);
	TEST_SUCC(ioctl(vhost_fd, VHOST_SET_MEM_TABLE, &table.memory));
}
END_TEST()

FN_TEST(vring_call_eventfd_can_be_unbound)
{
	struct vhost_vring_file file = {
		.index = 0,
		.fd = VHOST_FILE_UNBIND,
	};

	reset_fixture();
	configure_vhost_device_without_running(&fixture, GUEST_CID);
	TEST_SUCC(ioctl(fixture.fd, VHOST_SET_VRING_CALL, &file));
	start_vhost_device(&fixture);
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(reconfigure_while_running)
{
	uint64_t features = 0;
	uint64_t guest_cid = GUEST_CID + 1;
	struct vhost_vring_state state = {
		.index = 0,
		.num = RING_SIZE,
	};
	struct vhost_vring_addr addr = {
		.index = 0,
		.desc_user_addr = (uintptr_t)fixture.rx.desc,
		.avail_user_addr = (uintptr_t)&fixture.rx.avail,
		.used_user_addr = (uintptr_t)&fixture.rx.used,
	};
	struct vhost_vring_file file = {
		.index = 0,
		.fd = fixture.rx_kick,
	};

	reset_fixture();
	configure_vhost_device(&fixture, GUEST_CID);
	file.fd = fixture.rx_kick;

	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_FEATURES, &features), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_BACKEND_FEATURES, &features),
		   EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_MEM_TABLE, &fixture.mem.memory),
		   EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_NUM, &state), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_BASE, &state), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_ADDR, &addr), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_KICK, &file), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_CALL, &file), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_SET_VRING_ERR, &file), EBUSY);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_VSOCK_SET_GUEST_CID, &guest_cid),
		   EBUSY);

	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(vring_base_preserved_across_start_stop)
{
	struct vhost_vring_state state = { 0 };
	int running = 0;

	reset_fixture();
	configure_vhost_device_without_running(&fixture, GUEST_CID);

	state.index = 0;
	state.num = 3;
	TEST_SUCC(ioctl(fixture.fd, VHOST_SET_VRING_BASE, &state));
	state.index = 1;
	state.num = 5;
	TEST_SUCC(ioctl(fixture.fd, VHOST_SET_VRING_BASE, &state));

	start_vhost_device(&fixture);

	state.index = 0;
	state.num = 0;
	TEST_SUCC(ioctl(fixture.fd, VHOST_GET_VRING_BASE, &state));
	TEST_RES(state.num, state.num == 3);
	state.index = 1;
	state.num = 0;
	TEST_SUCC(ioctl(fixture.fd, VHOST_GET_VRING_BASE, &state));
	TEST_RES(state.num, state.num == 5);

	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	state.index = 0;
	state.num = 0;
	TEST_SUCC(ioctl(fixture.fd, VHOST_GET_VRING_BASE, &state));
	TEST_RES(state.num, state.num == 3);
	state.index = 1;
	state.num = 0;
	TEST_SUCC(ioctl(fixture.fd, VHOST_GET_VRING_BASE, &state));
	TEST_RES(state.num, state.num == 5);

	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(worker_failure_releases_guest_cid)
{
	int running = 0;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	configure_vhost_device(&fixture, GUEST_CID);

	fixture.tx.avail.ring[fixture.tx.avail.idx % RING_SIZE] = RING_SIZE;
	fixture.tx.avail.idx++;
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(tx_progress_preserved_after_partial_failure)
{
	struct vhost_vring_state state = {
		.index = 1,
	};
	int running = 0;

	reset_fixture();
	setup_tx_packet(&fixture, GUEST_CID + 1, PEER_PORT, PEER_PORT,
			VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	fixture.tx.avail.ring[fixture.tx.avail.idx % RING_SIZE] = RING_SIZE;
	fixture.tx.avail.idx++;

	configure_vhost_device(&fixture, GUEST_CID);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_RES(fixture.tx.used.idx, fixture.tx.used.idx == 1);
	TEST_RES(fixture.tx.used.ring[0].len, fixture.tx.used.ring[0].len == 0);

	TEST_SUCC(ioctl(fixture.fd, VHOST_GET_VRING_BASE, &state));
	TEST_RES(state.num, state.num == 1);

	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(tx_avail_delta_too_large_releases_guest_cid)
{
	int running = 0;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	fixture.tx.avail.idx = RING_SIZE + 1;

	configure_vhost_device(&fixture, GUEST_CID);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(rx_avail_delta_too_large_releases_guest_cid)
{
	int running = 0;
	int socket_fd;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	fixture.rx.avail.idx = RING_SIZE + 1;

	configure_vhost_device(&fixture, GUEST_CID);
	socket_fd = TEST_SUCC(connect_to_guest(PEER_PORT));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	TEST_SUCC(close(socket_fd));
	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(tx_descriptor_can_span_memory_regions)
{
	struct {
		struct vhost_memory memory;
		struct vhost_memory_region region[2];
	} table = { 0 };
	uint8_t *split_header = split_mem + PAGE_SIZE - 16;

	reset_fixture();
	memset(split_mem, 0, sizeof(split_mem));
	setup_tx_packet(&fixture, GUEST_CID + 1, PEER_PORT, PEER_PORT,
			VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	memcpy(split_header, &fixture.tx_header, sizeof(fixture.tx_header));
	fixture.tx.desc[TX_HEADER_DESC].addr = (uintptr_t)split_header;
	fixture.tx.desc[TX_HEADER_DESC].len = sizeof(fixture.tx_header);
	fixture.tx.desc[TX_HEADER_DESC].flags = 0;

	configure_vhost_device_without_running(&fixture, GUEST_CID);

	table.memory.nregions = 2;
	table.region[0].guest_phys_addr = (uintptr_t)split_mem;
	table.region[0].memory_size = PAGE_SIZE;
	table.region[0].userspace_addr = (uintptr_t)split_mem;
	table.region[1].guest_phys_addr = (uintptr_t)split_mem + PAGE_SIZE;
	table.region[1].memory_size = PAGE_SIZE;
	table.region[1].userspace_addr = (uintptr_t)split_mem + PAGE_SIZE;
	TEST_SUCC(ioctl(fixture.fd, VHOST_SET_MEM_TABLE, &table.memory));

	start_vhost_device(&fixture);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_RES(fixture.tx.used.idx, fixture.tx.used.idx == 1);
	TEST_RES(fixture.tx.used.ring[0].len, fixture.tx.used.ring[0].len == 0);

	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(indirect_tx_table_too_large_releases_guest_cid)
{
	int running = 0;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	setup_tx_packet(&fixture, GUEST_CID, PEER_PORT, PEER_PORT,
			VIRTIO_VSOCK_OP_RW, NULL, 0);
	fixture.tx.desc[TX_HEADER_DESC].addr = (uintptr_t)fixture.indirect_desc;
	fixture.tx.desc[TX_HEADER_DESC].len = INDIRECT_TABLE_LEN;
	fixture.tx.desc[TX_HEADER_DESC].flags = VRING_DESC_F_INDIRECT;
	fixture.indirect_desc[0].addr = (uintptr_t)&fixture.tx_header;
	fixture.indirect_desc[0].len = sizeof(fixture.tx_header);

	configure_vhost_device(&fixture, GUEST_CID);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(indirect_rx_table_too_large_releases_guest_cid)
{
	int running = 0;
	int socket_fd;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	fixture.rx.desc[RX_HEADER_DESC].addr = (uintptr_t)fixture.indirect_desc;
	fixture.rx.desc[RX_HEADER_DESC].len = INDIRECT_TABLE_LEN;
	fixture.rx.desc[RX_HEADER_DESC].flags = VRING_DESC_F_INDIRECT;
	fixture.rx.avail.ring[fixture.rx.avail.idx % RING_SIZE] = RX_HEAD;
	fixture.rx.avail.idx++;

	configure_vhost_device(&fixture, GUEST_CID);
	socket_fd = TEST_SUCC(connect_to_guest(PEER_PORT));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(wait_worker_stopped(fixture.fd));
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));

	TEST_SUCC(close(socket_fd));
	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(unregistered_guest_cid_connect)
{
	int socket_fd = socket(AF_VSOCK, SOCK_STREAM | SOCK_NONBLOCK, 0);
	struct sockaddr_vm addr = {
		.svm_family = AF_VSOCK,
		.svm_cid = GUEST_CID,
		.svm_port = PEER_PORT,
	};

	TEST_RES(socket_fd, socket_fd >= 0);
	TEST_ERRNO(connect(socket_fd, (struct sockaddr *)&addr, sizeof(addr)),
		   ENETUNREACH);
	CHECK(close(socket_fd));
}
END_TEST()

FN_TEST(host_cid_connect)
{
	int socket_fd = socket(AF_VSOCK, SOCK_STREAM | SOCK_NONBLOCK, 0);
	struct sockaddr_vm addr = {
		.svm_family = AF_VSOCK,
		.svm_cid = VMADDR_CID_HOST,
		.svm_port = PEER_PORT,
	};

	TEST_RES(socket_fd, socket_fd >= 0);
	TEST_ERRNO(connect(socket_fd, (struct sockaddr *)&addr, sizeof(addr)),
		   ENETUNREACH);
	CHECK(close(socket_fd));
}
END_TEST()

FN_TEST(duplicate_running_guest_cid)
{
	uint64_t guest_cid = 4;
	uint64_t other_guest_cid = 5;
	int running = 1;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	configure_vhost_device(&fixture, guest_cid);
	TEST_ERRNO(ioctl(fixture.fd, VHOST_VSOCK_SET_GUEST_CID,
			 &other_guest_cid),
		   EBUSY);

	configure_vhost_device_without_running(&second_fixture, guest_cid);
	TEST_ERRNO(ioctl(second_fixture.fd, VHOST_VSOCK_SET_RUNNING, &running),
		   EBUSY);

	running = 0;
	TEST_SUCC(ioctl(fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));
	running = 1;
	TEST_SUCC(ioctl(second_fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));
	running = 0;
	TEST_SUCC(ioctl(second_fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));
	running = 1;
	TEST_SUCC(ioctl(second_fixture.fd, VHOST_VSOCK_SET_RUNNING, &running));
	TEST_SUCC(teardown_vhost_device(&second_fixture));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(reset_with_pending_connect_releases_guest_cid)
{
	int socket_fd;

	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	configure_vhost_device(&fixture, GUEST_CID);
	socket_fd = TEST_SUCC(connect_to_guest(PEER_PORT));

	TEST_SUCC(teardown_vhost_device(&fixture));
	TEST_SUCC(close(socket_fd));
	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
}
END_TEST()

FN_TEST(drop_releases_running_guest_cid)
{
	reset_fixture();
	reset_vhost_fixture(&second_fixture);
	configure_vhost_device(&fixture, GUEST_CID);

	close_fd_if_open(&fixture.fd);
	close_vhost_eventfds(&fixture);

	configure_vhost_device(&second_fixture, GUEST_CID);
	TEST_SUCC(teardown_vhost_device(&second_fixture));
}
END_TEST()

FN_TEST(connect_send_and_cid_validation)
{
	const char payload[] = "host-to-guest";
	int socket_fd;
	uint32_t host_port;

	reset_fixture();
	setup_rx_buffer_chain(&fixture);
	configure_vhost_device(&fixture, GUEST_CID);

	socket_fd = TEST_SUCC(connect_to_guest(PEER_PORT));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(expect_rx_packet(&fixture, VIRTIO_VSOCK_OP_REQUEST, PEER_PORT,
				   NULL));
	host_port = fixture.rx_header.src_port;

	setup_tx_packet(&fixture, GUEST_CID + 1, PEER_PORT, host_port,
			VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_RES(fixture.tx.used.idx, fixture.tx.used.idx == 1);
	TEST_RES(fixture.tx.used.ring[0].len, fixture.tx.used.ring[0].len == 0);

	setup_tx_packet(&fixture, GUEST_CID, PEER_PORT, host_port,
			VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_RES(fixture.tx.used.idx, fixture.tx.used.idx == 2);
	TEST_RES(fixture.tx.used.ring[1].len, fixture.tx.used.ring[1].len == 0);
	TEST_SUCC(wait_socket_writable(socket_fd));

	setup_rx_buffer_chain(&fixture);
	TEST_RES(send(socket_fd, payload, strlen(payload), 0),
		 _ret == (ssize_t)strlen(payload));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(expect_rx_packet(&fixture, VIRTIO_VSOCK_OP_RW, PEER_PORT,
				   payload));

	setup_rx_buffer_chain(&fixture);
	TEST_SUCC(shutdown(socket_fd, SHUT_RDWR));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(expect_rx_packet(&fixture, VIRTIO_VSOCK_OP_SHUTDOWN,
				   PEER_PORT, NULL));
	setup_tx_packet(&fixture, GUEST_CID, PEER_PORT, host_port,
			VIRTIO_VSOCK_OP_RST, NULL, 0);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_RES(fixture.tx.used.idx, fixture.tx.used.idx == 3);
	TEST_RES(fixture.tx.used.ring[2].len, fixture.tx.used.ring[2].len == 0);

	TEST_SUCC(close(socket_fd));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()

FN_TEST(blocking_eventfds_connect_send)
{
	const char payload[] = "blocking-eventfd";
	int socket_fd;
	uint32_t host_port;

	reset_fixture();
	setup_rx_buffer_chain(&fixture);
	configure_vhost_device_without_running_with_eventfd_flags(
		&fixture, GUEST_CID, EFD_CLOEXEC);
	start_vhost_device(&fixture);

	socket_fd = TEST_SUCC(connect_to_guest(PEER_PORT));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(expect_rx_packet(&fixture, VIRTIO_VSOCK_OP_REQUEST, PEER_PORT,
				   NULL));
	host_port = fixture.rx_header.src_port;

	setup_tx_packet(&fixture, GUEST_CID, PEER_PORT, host_port,
			VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	TEST_SUCC(kick_eventfd(fixture.tx_kick));
	TEST_SUCC(wait_eventfd(fixture.tx_call));
	TEST_SUCC(wait_socket_writable(socket_fd));

	setup_rx_buffer_chain(&fixture);
	TEST_RES(send(socket_fd, payload, strlen(payload), 0),
		 _ret == (ssize_t)strlen(payload));
	TEST_SUCC(kick_eventfd(fixture.rx_kick));
	TEST_SUCC(expect_rx_packet(&fixture, VIRTIO_VSOCK_OP_RW, PEER_PORT,
				   payload));

	TEST_SUCC(close(socket_fd));
	TEST_SUCC(teardown_vhost_device(&fixture));
}
END_TEST()
