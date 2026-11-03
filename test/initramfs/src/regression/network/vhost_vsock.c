// SPDX-License-Identifier: MPL-2.0

#include <assert.h>
#include <endian.h>
#include <fcntl.h>
#include <linux/vhost.h>
#include <linux/virtio_config.h>
#include <linux/virtio_ring.h>
#include <linux/virtio_vsock.h>
#include <linux/vm_sockets.h>
#include <poll.h>
#include <stdint.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#include "../common/test.h"

#define GUEST_CID 42
#define GUEST_PORT 4000
#define GUEST_MEMORY_BASE 0x100000
#define RING_SIZE 8
#define PAYLOAD_SIZE 128

struct test_ring {
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

struct guest_memory {
	struct test_ring rx;
	struct test_ring tx;
	struct vring_desc indirect[2];
	uint8_t rx_first[16];
	uint8_t gap[64];
	uint8_t rx_rest[sizeof(struct virtio_vsock_hdr) - 16 + PAYLOAD_SIZE];
	struct virtio_vsock_hdr tx_header;
	uint8_t tx_payload[PAYLOAD_SIZE];
};

struct backend {
	struct guest_memory *memory;
	size_t memory_size;
	int fd;
	int kick[2];
	int call[2];
	int error[2];
};

static uint64_t guest_address(struct backend *backend, const void *ptr)
{
	return GUEST_MEMORY_BASE +
	       ((const uint8_t *)ptr - (const uint8_t *)backend->memory);
}

static void configure_backend(struct backend *backend, int assign_cid,
			      int attach_events)
{
	long page_size = CHECK(sysconf(_SC_PAGESIZE));
	backend->memory_size = (sizeof(struct guest_memory) + page_size - 1) /
			       page_size * page_size;
	backend->memory = CHECK_WITH(mmap(NULL, backend->memory_size,
					  PROT_READ | PROT_WRITE,
					  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
				     _ret != MAP_FAILED);
	backend->fd = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	CHECK(ioctl(backend->fd, VHOST_SET_OWNER));
	uint64_t features = (1ULL << VIRTIO_F_VERSION_1) |
			    (1ULL << VIRTIO_RING_F_INDIRECT_DESC);
	CHECK(ioctl(backend->fd, VHOST_SET_FEATURES, &features));
	struct {
		struct vhost_memory table;
		struct vhost_memory_region region;
	} memory = {
		.table.nregions = 1,
		.region = {
			.guest_phys_addr = GUEST_MEMORY_BASE,
			.memory_size = backend->memory_size,
			.userspace_addr = (uintptr_t)backend->memory,
		},
	};
	CHECK(ioctl(backend->fd, VHOST_SET_MEM_TABLE, &memory));

	for (unsigned i = 0; i < 2; ++i) {
		struct test_ring *ring = i == 0 ? &backend->memory->rx :
						  &backend->memory->tx;
		struct vhost_vring_state state = { .index = i,
						   .num = RING_SIZE };
		CHECK(ioctl(backend->fd, VHOST_SET_VRING_NUM, &state));
		state.num = 0;
		CHECK(ioctl(backend->fd, VHOST_SET_VRING_BASE, &state));
		struct vhost_vring_addr addr = {
			.index = i,
			.desc_user_addr = (uintptr_t)ring->desc,
			.avail_user_addr = (uintptr_t)&ring->avail,
			.used_user_addr = (uintptr_t)&ring->used,
		};
		CHECK(ioctl(backend->fd, VHOST_SET_VRING_ADDR, &addr));
		backend->kick[i] =
			CHECK(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK));
		backend->call[i] =
			CHECK(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK));
		struct vhost_vring_file file = { .index = i,
						 .fd = backend->kick[i] };
		if (attach_events)
			CHECK(ioctl(backend->fd, VHOST_SET_VRING_KICK, &file));
		file.fd = backend->call[i];
		if (attach_events)
			CHECK(ioctl(backend->fd, VHOST_SET_VRING_CALL, &file));
		backend->error[i] =
			CHECK(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK));
		file.fd = backend->error[i];
		if (attach_events)
			CHECK(ioctl(backend->fd, VHOST_SET_VRING_ERR, &file));
	}
	uint64_t cid = GUEST_CID;
	if (assign_cid)
		CHECK(ioctl(backend->fd, VHOST_VSOCK_SET_GUEST_CID, &cid));
	int running = 1;
	CHECK(ioctl(backend->fd, VHOST_VSOCK_SET_RUNNING, &running));
}

static void setup_backend(struct backend *backend)
{
	configure_backend(backend, 1, 1);
}

static void destroy_backend(struct backend *backend)
{
	CHECK(close(backend->fd));
	for (unsigned i = 0; i < 2; ++i) {
		CHECK(close(backend->kick[i]));
		CHECK(close(backend->call[i]));
		CHECK(close(backend->error[i]));
	}
	CHECK(munmap(backend->memory, backend->memory_size));
}

static void publish(struct test_ring *ring, int kick)
{
	uint16_t index = le16toh(ring->avail.idx);
	ring->avail.ring[index % RING_SIZE] = 0;
	__atomic_store_n(&ring->avail.idx, htole16(index + 1),
			 __ATOMIC_RELEASE);
	uint64_t one = 1;
	CHECK_WITH(write(kick, &one, sizeof(one)), _ret == sizeof(one));
}

static void wait_used(struct test_ring *ring, int call, uint32_t length)
{
	struct pollfd pfd = { .fd = call, .events = POLLIN };
	struct timespec deadline;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &deadline));
	deadline.tv_sec += 5;
	uint16_t index = le16toh(ring->avail.idx);
	while (le16toh(__atomic_load_n(&ring->used.idx, __ATOMIC_ACQUIRE)) !=
	       index) {
		struct timespec now;
		CHECK(clock_gettime(CLOCK_MONOTONIC, &now));
		long timeout = (deadline.tv_sec - now.tv_sec) * 1000 +
			       (deadline.tv_nsec - now.tv_nsec) / 1000000;
		assert(timeout > 0);
		CHECK_WITH(poll(&pfd, 1, timeout),
			   _ret == 1 && (pfd.revents & POLLIN));
		uint64_t count;
		CHECK_WITH(read(call, &count, sizeof(count)),
			   _ret == sizeof(count));
	}
	assert(le32toh(ring->used.ring[(index - 1) % RING_SIZE].id) == 0);
	assert(le32toh(ring->used.ring[(index - 1) % RING_SIZE].len) == length);
}

static void offer_rx(struct backend *backend, size_t payload_size)
{
	struct guest_memory *memory = backend->memory;
	memory->indirect[0] = (struct vring_desc){
		.addr = htole64(guest_address(backend, memory->rx_first)),
		.len = htole32(sizeof(memory->rx_first)),
		.flags = htole16(VRING_DESC_F_WRITE | VRING_DESC_F_NEXT),
		.next = htole16(1),
	};
	memory->indirect[1] = (struct vring_desc){
		.addr = htole64(guest_address(backend, memory->rx_rest)),
		.len = htole32(sizeof(struct virtio_vsock_hdr) -
			       sizeof(memory->rx_first) + payload_size),
		.flags = htole16(VRING_DESC_F_WRITE),
	};
	// The 32-byte indirect table describes a header split across two buffers.
	memory->rx.desc[0] = (struct vring_desc){
		.addr = htole64(guest_address(backend, memory->indirect)),
		.len = htole32(sizeof(memory->indirect)),
		.flags = htole16(VRING_DESC_F_INDIRECT),
	};
	publish(&memory->rx, backend->kick[0]);
}

static struct virtio_vsock_hdr receive_packet(struct backend *backend,
					      uint16_t op, size_t length)
{
	struct guest_memory *memory = backend->memory;
	struct virtio_vsock_hdr header;
	wait_used(&memory->rx, backend->call[0], sizeof(header) + length);
	memcpy(&header, memory->rx_first, sizeof(memory->rx_first));
	memcpy((uint8_t *)&header + sizeof(memory->rx_first), memory->rx_rest,
	       sizeof(header) - sizeof(memory->rx_first));
	assert(le64toh(header.src_cid) == VMADDR_CID_HOST);
	assert(le64toh(header.dst_cid) == GUEST_CID);
	assert(le32toh(header.dst_port) == GUEST_PORT);
	assert(le16toh(header.type) == VIRTIO_VSOCK_TYPE_STREAM);
	assert(le16toh(header.op) == op);
	assert(le32toh(header.len) == length);
	return header;
}

static void submit_packet(struct backend *backend, uint32_t host_port,
			  uint16_t op, const void *payload, size_t length)
{
	struct guest_memory *memory = backend->memory;
	memory->tx_header = (struct virtio_vsock_hdr){
		.src_cid = htole64(GUEST_CID),
		.dst_cid = htole64(VMADDR_CID_HOST),
		.src_port = htole32(GUEST_PORT),
		.dst_port = htole32(host_port),
		.len = htole32(length),
		.type = htole16(VIRTIO_VSOCK_TYPE_STREAM),
		.op = htole16(op),
		.buf_alloc = htole32(256 * 1024),
	};
	memory->tx.desc[0] = (struct vring_desc){
		.addr = htole64(guest_address(backend, &memory->tx_header)),
		.len = htole32(sizeof(memory->tx_header)),
		.flags = htole16(length ? VRING_DESC_F_NEXT : 0),
		.next = htole16(1),
	};
	if (length) {
		assert(length <= sizeof(memory->tx_payload));
		memcpy(memory->tx_payload, payload, length);
		memory->tx.desc[1] = (struct vring_desc){
			.addr = htole64(
				guest_address(backend, memory->tx_payload)),
			.len = htole32(length),
		};
	}
	publish(&memory->tx, backend->kick[1]);
}

static void send_packet(struct backend *backend, uint32_t host_port,
			uint16_t op, const void *payload, size_t length)
{
	submit_packet(backend, host_port, op, payload, length);
	// TX descriptors are read-only, so completion never reports bytes written.
	wait_used(&backend->memory->tx, backend->call[1], 0);
}

// Check completion independently of call notification for suppression/detach.
static void wait_used_without_call(struct test_ring *ring)
{
	struct timespec deadline;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &deadline));
	deadline.tv_sec += 5;
	while (le16toh(__atomic_load_n(&ring->used.idx, __ATOMIC_ACQUIRE)) !=
	       le16toh(ring->avail.idx)) {
		struct timespec now;
		CHECK(clock_gettime(CLOCK_MONOTONIC, &now));
		assert(now.tv_sec < deadline.tv_sec ||
		       (now.tv_sec == deadline.tv_sec &&
			now.tv_nsec < deadline.tv_nsec));
		CHECK(usleep(1000));
	}
}

static int connect_guest(void)
{
	int fd = CHECK(socket(AF_VSOCK, SOCK_STREAM | SOCK_NONBLOCK, 0));
	struct sockaddr_vm addr = {
		.svm_family = AF_VSOCK,
		.svm_cid = GUEST_CID,
		.svm_port = GUEST_PORT,
	};
	CHECK_WITH(connect(fd, (struct sockaddr *)&addr, sizeof(addr)),
		   _ret == -1 && errno == EINPROGRESS);
	return fd;
}

FN_TEST(cid_owner_and_close)
{
	uint64_t cid = GUEST_CID;
	int first = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	int second = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));

	CHECK(ioctl(first, VHOST_SET_OWNER));
	CHECK(ioctl(second, VHOST_SET_OWNER));
	uint64_t reserved = VMADDR_CID_HOST;
	TEST_ERRNO(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &reserved), EINVAL);
	TEST_SUCC(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &cid));
	TEST_ERRNO(ioctl(second, VHOST_VSOCK_SET_GUEST_CID, &cid), EADDRINUSE);
#ifdef __asterinas__
	TEST_ERRNO(ioctl(first, VHOST_RESET_OWNER, NULL), ENOTTY);
#else
	TEST_ERRNO(ioctl(first, VHOST_RESET_OWNER, NULL), EFAULT);
#endif
	CHECK(close(first));
	TEST_SUCC(ioctl(second, VHOST_VSOCK_SET_GUEST_CID, &cid));
	CHECK(close(second));
	first = CHECK(open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC));
	CHECK(ioctl(first, VHOST_SET_OWNER));
	TEST_SUCC(ioctl(first, VHOST_VSOCK_SET_GUEST_CID, &cid));
	CHECK(close(first));
}
END_TEST()

FN_TEST(indirect_rx_and_bidirectional_stream)
{
	struct backend backend;
	setup_backend(&backend);
	offer_rx(&backend, 0);
	int fd = connect_guest();
	struct virtio_vsock_hdr header =
		receive_packet(&backend, VIRTIO_VSOCK_OP_REQUEST, 0);
	uint32_t host_port = le32toh(header.src_port);
	send_packet(&backend, host_port, VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	struct pollfd pfd = { .fd = fd, .events = POLLOUT };
	TEST_RES(poll(&pfd, 1, 5000), _ret == 1 && (pfd.revents & POLLOUT));
	int option = -1;
	socklen_t option_len = sizeof(option);
	TEST_RES(getsockopt(fd, SOL_SOCKET, SO_ERROR, &option, &option_len),
		 _ret == 0 && option == 0);
	TEST_RES(getsockopt(fd, SOL_SOCKET, SO_TYPE, &option, &option_len),
		 _ret == 0 && option == SOCK_STREAM);

	int running = 0;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	const char outgoing[] = "host-to-guest";
	TEST_RES(send(fd, outgoing, sizeof(outgoing), 0),
		 _ret == sizeof(outgoing));
	running = 1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	// Pausing preserves pending data; small RX chains split it into packets.
	char sent[sizeof(outgoing)];
	for (size_t offset = 0; offset < sizeof(sent);) {
		size_t length = sizeof(sent) - offset;
		if (length > 5)
			length = 5;
		offer_rx(&backend, length);
		receive_packet(&backend, VIRTIO_VSOCK_OP_RW, length);
		memcpy(sent + offset,
		       backend.memory->rx_rest + sizeof(header) -
			       sizeof(backend.memory->rx_first),
		       length);
		offset += length;
	}
	TEST_RES(memcmp(sent, outgoing, sizeof(outgoing)), _ret == 0);

	const char incoming[] = "guest-to-host";
	send_packet(&backend, host_port, VIRTIO_VSOCK_OP_RW, incoming,
		    sizeof(incoming));
	char received[sizeof(incoming)];
	TEST_RES(recv(fd, received, sizeof(received), 0),
		 _ret == sizeof(received));
	TEST_RES(memcmp(received, incoming, sizeof(incoming)), _ret == 0);
	CHECK(close(fd));
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(connection_reset_and_stop_restart)
{
	struct backend backend;
	setup_backend(&backend);
	offer_rx(&backend, 0);
	int fd = connect_guest();
	struct virtio_vsock_hdr header =
		receive_packet(&backend, VIRTIO_VSOCK_OP_REQUEST, 0);
	send_packet(&backend, le32toh(header.src_port), VIRTIO_VSOCK_OP_RST,
		    NULL, 0);
	struct pollfd pfd = { .fd = fd, .events = POLLOUT };
	TEST_RES(poll(&pfd, 1, 5000), _ret == 1);
	int error = 0;
	socklen_t error_len = sizeof(error);
	TEST_RES(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &error_len),
		 _ret == 0 && error == ECONNRESET);
	CHECK(close(fd));

	int running = 0;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	struct vhost_vring_state state = { .index = 1 };
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 1);
	running = 1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	offer_rx(&backend, 0);
	fd = connect_guest();
	receive_packet(&backend, VIRTIO_VSOCK_OP_REQUEST, 0);
	CHECK(close(fd));
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(owner_memory_fault_signals_error)
{
	struct backend backend;
	setup_backend(&backend);
	int fd = connect_guest();
	CHECK(mprotect(backend.memory, backend.memory_size, PROT_NONE));
	uint64_t one = 1;
	CHECK_WITH(write(backend.kick[0], &one, sizeof(one)),
		   _ret == sizeof(one));
	struct pollfd errors[] = {
		{ .fd = backend.error[0], .events = POLLIN },
		{ .fd = backend.error[1], .events = POLLIN },
	};
	TEST_RES(poll(errors, 2, 5000),
		 _ret > 0 &&
			 ((errors[0].revents | errors[1].revents) & POLLIN));
	CHECK(mprotect(backend.memory, backend.memory_size,
		       PROT_READ | PROT_WRITE));
	assert(backend.memory->rx.used.idx == 0);
	assert(backend.memory->tx.used.idx == 0);
	offer_rx(&backend, 0);
	receive_packet(&backend, VIRTIO_VSOCK_OP_REQUEST, 0);
	CHECK(close(fd));
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(running_reconfiguration)
{
	struct backend backend;
	setup_backend(&backend);
	int running = 2;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	running = -1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	struct vhost_vring_state state = { .index = 1, .num = RING_SIZE };
	TEST_ERRNO(ioctl(backend.fd, VHOST_SET_VRING_NUM, &state), EBUSY);
	state.num = 0;
	TEST_ERRNO(ioctl(backend.fd, VHOST_SET_VRING_BASE, &state), EBUSY);
	state.index = 2;
	TEST_ERRNO(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state), ENOBUFS);
	state.index = 1;
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 0);
	// A reset packet to an unknown socket needs no RX buffer or connection.
	send_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 1);
	struct vhost_vring_addr addr = {
		.index = 1,
		.desc_user_addr = (uintptr_t)backend.memory->tx.desc,
		.avail_user_addr = (uintptr_t)&backend.memory->tx.avail,
		.used_user_addr = (uintptr_t)&backend.memory->tx.used,
		.log_guest_addr = GUEST_MEMORY_BASE,
	};
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_VRING_ADDR, &addr));
	addr.used_user_addr = UINT64_MAX - 3;
	TEST_ERRNO(ioctl(backend.fd, VHOST_SET_VRING_ADDR, &addr), EINVAL);
	struct {
		struct vhost_memory table;
		struct vhost_memory_region region;
	} memory = {
		.table.nregions = 1,
		.region = {
			.guest_phys_addr = GUEST_MEMORY_BASE,
			.memory_size = backend.memory_size,
			.userspace_addr = (uintptr_t)backend.memory,
		},
	};
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_MEM_TABLE, &memory));
	memory.region.userspace_addr = UINT64_MAX;
	TEST_ERRNO(ioctl(backend.fd, VHOST_SET_MEM_TABLE, &memory), EFAULT);
	// Both failed updates and GET_BASE leave the original queue running.
	send_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 2);
	for (unsigned i = 0; i < 2; ++i) {
		int kick = CHECK(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK));
		struct vhost_vring_file file = { .index = i, .fd = kick };
		TEST_SUCC(ioctl(backend.fd, VHOST_SET_VRING_KICK, &file));
		CHECK(close(backend.kick[i]));
		backend.kick[i] = kick;
	}
	send_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	running = 0;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	running = 1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	send_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 4);
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(start_without_cid_or_events)
{
	struct backend backend;
	configure_backend(&backend, 0, 0);
	int running = 0;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	struct vhost_memory memory = { .nregions = 0 };
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_MEM_TABLE, &memory));
	running = 1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(cid_change_and_close_reset_orphans)
{
	struct backend backend;
	setup_backend(&backend);
	offer_rx(&backend, 0);
	int fd = connect_guest();
	struct virtio_vsock_hdr header =
		receive_packet(&backend, VIRTIO_VSOCK_OP_REQUEST, 0);
	send_packet(&backend, le32toh(header.src_port),
		    VIRTIO_VSOCK_OP_RESPONSE, NULL, 0);
	struct pollfd pfd = { .fd = fd, .events = POLLOUT };
	CHECK_WITH(poll(&pfd, 1, 5000), _ret == 1);
	uint64_t cid = GUEST_CID + 1;
	TEST_SUCC(ioctl(backend.fd, VHOST_VSOCK_SET_GUEST_CID, &cid));
	struct vhost_vring_state state = { .index = 1 };
	TEST_RES(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state),
		 _ret == 0 && state.num == 1);
	int error = -1;
	socklen_t error_len = sizeof(error);
	TEST_RES(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &error_len),
		 _ret == 0 && error == 0);
	destroy_backend(&backend);
	pfd.events = POLLIN;
	TEST_RES(poll(&pfd, 1, 5000), _ret == 1);
	TEST_RES(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &error_len),
		 _ret == 0 && error == ECONNRESET);
	CHECK(close(fd));
}
END_TEST()

FN_TEST(optional_eventfd_bindings)
{
	struct backend backend;
	configure_backend(&backend, 1, 0);
	int non_event = CHECK(open("/dev/null", O_RDONLY));
	const unsigned long commands[] = {
		VHOST_SET_VRING_KICK,
		VHOST_SET_VRING_CALL,
		VHOST_SET_VRING_ERR,
	};
	for (unsigned i = 0; i < sizeof(commands) / sizeof(commands[0]); ++i) {
		struct vhost_vring_file file = { .index = 1, .fd = -1 };
		TEST_SUCC(ioctl(backend.fd, commands[i], &file));
		file.fd = -2;
		TEST_ERRNO(ioctl(backend.fd, commands[i], &file), EBADF);
		file.fd = non_event;
		TEST_ERRNO(ioctl(backend.fd, commands[i], &file), EINVAL);
		file.fd = backend.kick[1];
		TEST_SUCC(ioctl(backend.fd, commands[i], &file));
		file.fd = -1;
		TEST_SUCC(ioctl(backend.fd, commands[i], &file));
	}
	CHECK(close(non_event));
	destroy_backend(&backend);
}
END_TEST()

FN_TEST(call_notification_suppression_and_rebinding)
{
	struct backend backend;
	setup_backend(&backend);
	struct test_ring *ring = &backend.memory->tx;
	uint64_t count;

	ring->avail.flags = htole16(VRING_AVAIL_F_NO_INTERRUPT);
	submit_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	wait_used_without_call(ring);
	// Wait for the batch's notification decision before changing avail.flags.
	struct vhost_vring_state state = { .index = 1 };
	CHECK(ioctl(backend.fd, VHOST_GET_VRING_BASE, &state));
	TEST_ERRNO(read(backend.call[1], &count, sizeof(count)), EAGAIN);

	ring->avail.flags = 0;
	submit_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	struct pollfd pfd = { .fd = backend.call[1], .events = POLLIN };
	TEST_RES(poll(&pfd, 1, 5000), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(pfd.fd, &count, sizeof(count)),
		 _ret == sizeof(count) && count > 0);
	wait_used_without_call(ring);

	struct vhost_vring_file file = { .index = 1, .fd = -1 };
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_VRING_CALL, &file));
	submit_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	wait_used_without_call(ring);
	TEST_ERRNO(read(backend.call[1], &count, sizeof(count)), EAGAIN);

	int replacement = CHECK(eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK));
	file.fd = replacement;
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_VRING_CALL, &file));
	submit_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	pfd.fd = replacement;
	TEST_RES(poll(&pfd, 1, 5000), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(pfd.fd, &count, sizeof(count)),
		 _ret == sizeof(count) && count > 0);
	wait_used_without_call(ring);
	TEST_ERRNO(read(backend.call[1], &count, sizeof(count)), EAGAIN);
	CHECK(close(replacement));
	destroy_backend(&backend);
}
END_TEST()

// A vhost memory region is not required to end on a descriptor boundary.
// Linux's get_indirect reads through iov_iter across adjacent mappings:
// https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.c#L2723
FN_TEST(indirect_descriptor_crosses_memory_regions)
{
	struct backend backend;
	setup_backend(&backend);
	int running = 0;
	CHECK(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	submit_packet(&backend, 6000, VIRTIO_VSOCK_OP_RST, NULL, 0);
	struct guest_memory *guest = backend.memory;
	guest->indirect[0] = guest->tx.desc[0];
	guest->tx.desc[0] = (struct vring_desc){
		.addr = htole64(guest_address(&backend, guest->indirect)),
		.len = htole32(sizeof(struct vring_desc)),
		.flags = htole16(VRING_DESC_F_INDIRECT),
	};
	// Put the latter eight bytes of the descriptor in a separate host mapping.
	size_t split = (uint8_t *)guest->indirect - (uint8_t *)guest + 8;
	size_t tail_size = backend.memory_size - split;
	void *tail = CHECK_WITH(mmap(NULL, tail_size, PROT_READ | PROT_WRITE,
				     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
				_ret != MAP_FAILED);
	memcpy(tail, (uint8_t *)guest + split, tail_size);
	struct {
		struct vhost_memory table;
		struct vhost_memory_region regions[2];
	} memory = {
		.table.nregions = 2,
		.regions = {
			{
				.guest_phys_addr = GUEST_MEMORY_BASE,
				.memory_size = split,
				.userspace_addr = (uintptr_t)guest,
			},
			{
				.guest_phys_addr = GUEST_MEMORY_BASE + split,
				.memory_size = tail_size,
				.userspace_addr = (uintptr_t)tail,
			},
		},
	};
	TEST_SUCC(ioctl(backend.fd, VHOST_SET_MEM_TABLE, &memory));
	running = 1;
	CHECK(ioctl(backend.fd, VHOST_VSOCK_SET_RUNNING, &running));
	// Linux may have consumed the earlier kick while TX was stopped.
	uint64_t one = 1;
	CHECK_WITH(write(backend.kick[1], &one, sizeof(one)),
		   _ret == sizeof(one));
	wait_used(&guest->tx, backend.call[1], 0);
	TEST_RES(le16toh(guest->tx.used.idx), _ret == 1);
	destroy_backend(&backend);
	CHECK(munmap(tail, tail_size));
}
END_TEST()
