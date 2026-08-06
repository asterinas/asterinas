// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <endian.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <inttypes.h>
#include <sys/socket.h>
#include <linux/vhost.h>
#include <linux/virtio_ring.h>
#include <linux/virtio_vsock.h>
#include <linux/vm_sockets.h>
#include <poll.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/uio.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

/*
 * The workload shape follows Linux tools/testing/vsock/vsock_perf.c: measure
 * one direction at a time, after warmup, with fixed byte and buffer sizes.
 * Unlike vsock_perf, this program drives /dev/vhost-vsock directly and emulates
 * the guest-side split virtqueues, so the same binary runs on Linux and
 * Asterinas without a nested VM or VMM.
 */

#define GUEST_CID UINT64_C(42)
#define GUEST_PORT UINT32_C(4000)
#define QUEUE_SIZE 256U
#define MAX_PAYLOAD_SIZE (64U * 1024U)
#define VSOCK_BUFFER_SIZE (256U * 1024U)
#define CREDIT_UPDATE_THRESHOLD (VSOCK_BUFFER_SIZE / 4U)
#define IO_TIMEOUT_MS 30000
#define PAGE_SIZE 4096U
#define DEFAULT_TOTAL_BYTES (16U * 1024U * 1024U)
#define DEFAULT_WARMUP_BYTES (1U * 1024U * 1024U)
#define ALIGN_UP(value, align) (((value) + (align) - 1U) & ~((align) - 1U))

#define VHOST_USER_VERSION 1U
#define VHOST_USER_REPLY_MASK 0x4U
#define VHOST_USER_F_PROTOCOL_FEATURES (UINT64_C(1) << 30)
#define VIRTIO_F_VERSION_1_MASK (UINT64_C(1) << VIRTIO_F_VERSION_1)
#define VHOST_USER_PROTOCOL_F_MQ UINT64_C(1)

enum vhost_user_request {
  VHOST_USER_GET_FEATURES = 1,
  VHOST_USER_SET_FEATURES = 2,
  VHOST_USER_SET_OWNER = 3,
  VHOST_USER_SET_MEM_TABLE = 5,
  VHOST_USER_SET_VRING_NUM = 8,
  VHOST_USER_SET_VRING_ADDR = 9,
  VHOST_USER_SET_VRING_BASE = 10,
  VHOST_USER_SET_VRING_KICK = 12,
  VHOST_USER_SET_VRING_CALL = 13,
  VHOST_USER_GET_PROTOCOL_FEATURES = 15,
  VHOST_USER_SET_PROTOCOL_FEATURES = 16,
  VHOST_USER_GET_QUEUE_NUM = 17,
  VHOST_USER_SET_VRING_ENABLE = 18,
};

enum backend_type {
  BACKEND_VHOST,
  BACKEND_VHOST_USER,
};

enum direction {
  DIRECTION_H2G,
  DIRECTION_G2H,
};

struct avail_ring {
  uint16_t flags;
  uint16_t idx;
  uint16_t ring[QUEUE_SIZE];
  uint16_t used_event;
} __attribute__((aligned(2)));

struct used_ring {
  uint16_t flags;
  uint16_t idx;
  struct vring_used_elem ring[QUEUE_SIZE];
  uint16_t avail_event;
} __attribute__((aligned(4)));

struct split_ring {
  struct vring_desc desc[QUEUE_SIZE];
  struct avail_ring avail;
  struct used_ring used;
};

struct packet_buffer {
  struct virtio_vsock_hdr header;
  uint8_t payload[MAX_PAYLOAD_SIZE];
};

struct guest_memory {
  struct split_ring rx;
  struct split_ring tx;
  struct packet_buffer rx_packets[QUEUE_SIZE];
  struct packet_buffer tx_packets[QUEUE_SIZE];
};

union guest_memory_pages {
  struct guest_memory state;
  uint8_t pages[ALIGN_UP(sizeof(struct guest_memory), PAGE_SIZE)];
} __attribute__((aligned(PAGE_SIZE)));

struct metrics {
  uint64_t rx_kicks;
  uint64_t tx_kicks;
  uint64_t rx_calls;
  uint64_t tx_calls;
  uint64_t rx_data_packets;
  uint64_t tx_data_packets;
  uint64_t rx_control_packets;
  uint64_t tx_control_packets;
};

struct vhost_user_header {
  uint32_t request;
  uint32_t flags;
  uint32_t size;
} __attribute__((packed));

struct vhost_user_memory {
  uint32_t num_regions;
  uint32_t padding;
} __attribute__((packed));

struct vhost_user_memory_region {
  uint64_t guest_phys_addr;
  uint64_t memory_size;
  uint64_t user_addr;
  uint64_t mmap_offset;
} __attribute__((packed));

struct vhost_user_memory_table {
  struct vhost_user_memory memory;
  struct vhost_user_memory_region region;
} __attribute__((packed));

struct vhost_user_vring_state {
  uint32_t index;
  uint32_t num;
} __attribute__((packed));

struct vhost_user_vring_addr {
  uint32_t index;
  uint32_t flags;
  uint64_t descriptor;
  uint64_t used;
  uint64_t available;
  uint64_t log;
} __attribute__((packed));

struct vhost_device {
  union guest_memory_pages *memory;
  struct {
    struct vhost_memory memory;
    struct vhost_memory_region region;
  } memory_table;
  enum backend_type backend;
  int fd;
  int memory_fd;
  int rx_kick;
  int rx_call;
  int tx_kick;
  int tx_call;
  uint16_t rx_avail_idx;
  uint16_t rx_used_idx;
  uint16_t tx_avail_idx;
  uint16_t tx_used_idx;
  uint32_t payload_size;
  uint32_t host_port;
  uint32_t host_buf_alloc;
  uint32_t host_fwd_cnt;
  uint32_t guest_tx_cnt;
  uint32_t guest_fwd_cnt;
  uint32_t last_credit_update;
  struct metrics metrics;
};

struct host_io {
  int socket_fd;
  int start_fd;
  enum direction direction;
  uint8_t *buffer;
  uint32_t buffer_size;
  uint64_t total_bytes;
  uint64_t calls;
  _Atomic uint64_t started_ns;
  _Atomic uint64_t completed_ns;
};

struct received_packet {
  struct virtio_vsock_hdr header;
  uint32_t payload_len;
};

static void fatal(const char *format, ...) {
  int saved_errno = errno;
  va_list args;

  fprintf(stderr, "vhost-vsock-bench: ");
  va_start(args, format);
  vfprintf(stderr, format, args);
  va_end(args);
  if (saved_errno != 0)
    fprintf(stderr, ": %s", strerror(saved_errno));
  fputc('\n', stderr);
  exit(EXIT_FAILURE);
}

static void checked_ioctl(int fd, unsigned long request, void *argument,
                          const char *name) {
  if (ioctl(fd, request, argument) < 0)
    fatal("%s", name);
}

static void wait_for_fd(int fd, short events, const char *name);

static const char *backend_name(enum backend_type backend) {
  return backend == BACKEND_VHOST ? "vhost" : "vhost-user";
}

static void write_full(int fd, const void *buffer, size_t length,
                       const char *name) {
  const uint8_t *position = buffer;

  while (length != 0) {
    ssize_t written = write(fd, position, length);

    if (written > 0) {
      position += written;
      length -= (size_t)written;
      continue;
    }
    if (written < 0 && errno == EINTR)
      continue;
    if (written < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      wait_for_fd(fd, POLLOUT, name);
      continue;
    }
    if (written == 0)
      errno = EPIPE;
    fatal("write %s", name);
  }
}

static void read_full(int fd, void *buffer, size_t length, const char *name) {
  uint8_t *position = buffer;

  while (length != 0) {
    ssize_t received = read(fd, position, length);

    if (received > 0) {
      position += received;
      length -= (size_t)received;
      continue;
    }
    if (received < 0 && errno == EINTR)
      continue;
    if (received < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      wait_for_fd(fd, POLLIN, name);
      continue;
    }
    if (received == 0)
      errno = ECONNRESET;
    fatal("read %s", name);
  }
}

static uint64_t monotonic_nanoseconds(void) {
  struct timespec now;

  if (clock_gettime(CLOCK_MONOTONIC, &now) < 0)
    fatal("clock_gettime");
  return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

static uint64_t parse_size(const char *text, const char *name) {
  char *suffix;
  uint64_t multiplier = 1;
  unsigned long long value;

  errno = 0;
  value = strtoull(text, &suffix, 0);
  if (errno != 0 || suffix == text)
    fatal("invalid %s '%s'", name, text);

  if (*suffix != '\0') {
    switch (*suffix++) {
    case 'k':
    case 'K':
      multiplier = UINT64_C(1024);
      break;
    case 'm':
    case 'M':
      multiplier = UINT64_C(1024) * 1024;
      break;
    case 'g':
    case 'G':
      multiplier = UINT64_C(1024) * 1024 * 1024;
      break;
    default:
      fatal("invalid %s suffix in '%s'", name, text);
    }
    if (*suffix == 'i' || *suffix == 'I')
      suffix++;
    if (*suffix == 'b' || *suffix == 'B')
      suffix++;
    if (*suffix != '\0')
      fatal("invalid %s '%s'", name, text);
  }

  if ((uint64_t)value > UINT64_MAX / multiplier) {
    errno = ERANGE;
    fatal("%s is too large", name);
  }
  return (uint64_t)value * multiplier;
}

static void wait_for_fd(int fd, short events, const char *name) {
  struct pollfd poll_fd = {
      .fd = fd,
      .events = events,
  };
  int result;

  do {
    result = poll(&poll_fd, 1, IO_TIMEOUT_MS);
  } while (result < 0 && errno == EINTR);
  if (result == 0) {
    errno = ETIMEDOUT;
    fatal("timed out waiting for %s", name);
  }
  if (result < 0)
    fatal("poll %s", name);
  if ((poll_fd.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0 &&
      (poll_fd.revents & events) == 0) {
    errno = EIO;
    fatal("unexpected poll event for %s", name);
  }
}

static int connect_unix_socket(const char *path, const char *name) {
  struct sockaddr_un address = {
      .sun_family = AF_UNIX,
  };
  uint64_t deadline =
      monotonic_nanoseconds() + (uint64_t)IO_TIMEOUT_MS * UINT64_C(1000000);
  int fd;

  if (strlen(path) >= sizeof(address.sun_path)) {
    errno = ENAMETOOLONG;
    fatal("%s path", name);
  }
  strcpy(address.sun_path, path);
  for (;;) {
    fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
      fatal("create %s socket", name);
    if (connect(fd, (struct sockaddr *)&address, sizeof(address)) == 0)
      return fd;
    if (errno != ENOENT && errno != ECONNREFUSED) {
      int saved_errno = errno;

      close(fd);
      errno = saved_errno;
      fatal("connect %s socket", name);
    }
    if (close(fd) < 0)
      fatal("close unconnected %s socket", name);
    if (monotonic_nanoseconds() >= deadline) {
      errno = ETIMEDOUT;
      fatal("connect %s socket", name);
    }
    if (nanosleep(&(struct timespec){.tv_nsec = 10000000}, NULL) < 0 &&
        errno != EINTR)
      fatal("wait for %s socket", name);
  }
}

static void set_nonblocking(int fd, const char *name) {
  int flags = fcntl(fd, F_GETFL);

  if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0)
    fatal("make %s nonblocking", name);
}

static void send_vhost_user_message(int fd, enum vhost_user_request request,
                                    const void *payload, uint32_t payload_size,
                                    int passed_fd) {
  struct vhost_user_header header = {
      .request = request,
      .flags = VHOST_USER_VERSION,
      .size = payload_size,
  };
  struct iovec vectors[2] = {
      {.iov_base = &header, .iov_len = sizeof(header)},
      {.iov_base = (void *)payload, .iov_len = payload_size},
  };
  char control[CMSG_SPACE(sizeof(int))] = {0};
  struct msghdr message = {
      .msg_iov = vectors,
      .msg_iovlen = payload_size == 0 ? 1 : 2,
  };
  size_t expected = sizeof(header) + payload_size;
  ssize_t sent;

  if (passed_fd >= 0) {
    struct cmsghdr *control_header;

    message.msg_control = control;
    message.msg_controllen = sizeof(control);
    control_header = CMSG_FIRSTHDR(&message);
    control_header->cmsg_level = SOL_SOCKET;
    control_header->cmsg_type = SCM_RIGHTS;
    control_header->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(control_header), &passed_fd, sizeof(passed_fd));
  }

  do {
    sent = sendmsg(fd, &message, 0);
  } while (sent < 0 && errno == EINTR);
  if (sent < 0)
    fatal("send vhost-user request %u", request);
  if ((size_t)sent != expected) {
    errno = EIO;
    fatal("short vhost-user request %u", request);
  }
}

static uint64_t receive_vhost_user_u64(int fd,
                                       enum vhost_user_request request) {
  struct vhost_user_header header;
  uint64_t value;

  read_full(fd, &header, sizeof(header), "vhost-user reply header");
  if (header.request != (uint32_t)request ||
      header.flags != (VHOST_USER_VERSION | VHOST_USER_REPLY_MASK) ||
      header.size != sizeof(value)) {
    errno = EPROTO;
    fatal("invalid vhost-user reply to request %u", request);
  }
  read_full(fd, &value, sizeof(value), "vhost-user reply payload");
  return value;
}

static uint64_t get_vhost_user_u64(int fd, enum vhost_user_request request) {
  send_vhost_user_message(fd, request, NULL, 0, -1);
  return receive_vhost_user_u64(fd, request);
}

static void set_vhost_user_u64(int fd, enum vhost_user_request request,
                               uint64_t value) {
  send_vhost_user_message(fd, request, &value, sizeof(value), -1);
}

static uint64_t read_event_counter(int fd, const char *name) {
  uint64_t counter;
  ssize_t result;

  for (;;) {
    result = read(fd, &counter, sizeof(counter));
    if (result == (ssize_t)sizeof(counter))
      return counter;
    if (result < 0 && errno == EINTR)
      continue;
    if (result < 0 && errno == EAGAIN) {
      wait_for_fd(fd, POLLIN, name);
      continue;
    }
    fatal("read %s eventfd", name);
  }
}

static uint64_t drain_event_counter(int fd, const char *name) {
  uint64_t total = 0;

  for (;;) {
    uint64_t counter;
    ssize_t result = read(fd, &counter, sizeof(counter));

    if (result == (ssize_t)sizeof(counter)) {
      total += counter;
      continue;
    }
    if (result < 0 && errno == EINTR)
      continue;
    if (result < 0 && errno == EAGAIN)
      return total;
    fatal("drain %s eventfd", name);
  }
}

static void write_event_counter(int fd, const char *name) {
  uint64_t counter = 1;
  ssize_t result;

  do {
    result = write(fd, &counter, sizeof(counter));
  } while (result < 0 && errno == EINTR);
  if (result != (ssize_t)sizeof(counter))
    fatal("write %s eventfd", name);
}

static uint16_t load_index(const uint16_t *index) {
  return le16toh(__atomic_load_n(index, __ATOMIC_ACQUIRE));
}

static void store_index(uint16_t *index, uint16_t value) {
  __atomic_store_n(index, htole16(value), __ATOMIC_RELEASE);
}

static bool ring_wants_kick(const struct split_ring *ring) {
  uint16_t flags =
      le16toh(__atomic_load_n(&ring->used.flags, __ATOMIC_ACQUIRE));

  return (flags & VRING_USED_F_NO_NOTIFY) == 0;
}

static void kick_rx(struct vhost_device *device, bool force) {
  if (!force && !ring_wants_kick(&device->memory->state.rx))
    return;
  write_event_counter(device->rx_kick, "RX kick");
  device->metrics.rx_kicks++;
}

static void kick_tx(struct vhost_device *device, bool force) {
  if (!force && !ring_wants_kick(&device->memory->state.tx))
    return;
  write_event_counter(device->tx_kick, "TX kick");
  device->metrics.tx_kicks++;
}

static void publish_rx_buffer(struct vhost_device *device, uint16_t id) {
  struct split_ring *rx = &device->memory->state.rx;
  struct packet_buffer *packet = &device->memory->state.rx_packets[id];
  uint16_t slot = device->rx_avail_idx % QUEUE_SIZE;

  rx->desc[id].addr = htole64((uint64_t)(uintptr_t)packet);
  rx->desc[id].len = htole32(sizeof(packet->header) + device->payload_size);
  rx->desc[id].flags = htole16(VRING_DESC_F_WRITE);
  rx->desc[id].next = 0;
  rx->avail.ring[slot] = htole16(id);
  device->rx_avail_idx++;
  store_index(&rx->avail.idx, device->rx_avail_idx);
}

static void publish_initial_rx_buffers(struct vhost_device *device) {
  for (uint16_t id = 0; id < QUEUE_SIZE; id++)
    publish_rx_buffer(device, id);
}

static void wait_for_rx_used(struct vhost_device *device) {
  struct split_ring *rx = &device->memory->state.rx;

  while (load_index(&rx->used.idx) == device->rx_used_idx)
    device->metrics.rx_calls += read_event_counter(device->rx_call, "RX call");
}

static struct received_packet receive_packet(struct vhost_device *device) {
  struct split_ring *rx = &device->memory->state.rx;
  uint16_t slot;
  uint32_t id;
  uint32_t used_len;
  struct packet_buffer *packet;
  struct received_packet received;

  wait_for_rx_used(device);
  slot = device->rx_used_idx % QUEUE_SIZE;
  id = le32toh(rx->used.ring[slot].id);
  used_len = le32toh(rx->used.ring[slot].len);
  if (id >= QUEUE_SIZE || used_len < sizeof(packet->header)) {
    errno = EPROTO;
    fatal("invalid RX used element");
  }
  packet = &device->memory->state.rx_packets[id];
  __atomic_thread_fence(__ATOMIC_ACQUIRE);
  received.header = packet->header;
  received.payload_len = le32toh(packet->header.len);
  if (received.payload_len > device->payload_size ||
      used_len != sizeof(packet->header) + received.payload_len) {
    errno = EPROTO;
    fatal("invalid RX packet length");
  }

  device->rx_used_idx++;
  publish_rx_buffer(device, (uint16_t)id);
  kick_rx(device, false);
  return received;
}

static bool rx_packet_available(const struct vhost_device *device) {
  return load_index(&device->memory->state.rx.used.idx) != device->rx_used_idx;
}

static void fill_guest_header(struct vhost_device *device,
                              struct virtio_vsock_hdr *header, uint16_t op,
                              uint32_t payload_len) {
  memset(header, 0, sizeof(*header));
  header->src_cid = htole64(GUEST_CID);
  header->dst_cid = htole64(VMADDR_CID_HOST);
  header->src_port = htole32(GUEST_PORT);
  header->dst_port = htole32(device->host_port);
  header->len = htole32(payload_len);
  header->type = htole16(VIRTIO_VSOCK_TYPE_STREAM);
  header->op = htole16(op);
  header->buf_alloc = htole32(VSOCK_BUFFER_SIZE);
  header->fwd_cnt = htole32(device->guest_fwd_cnt);
}

static void submit_tx_packet(struct vhost_device *device, uint16_t op,
                             uint32_t payload_len) {
  struct split_ring *tx = &device->memory->state.tx;
  uint16_t id = device->tx_avail_idx % QUEUE_SIZE;
  uint16_t slot = id;
  struct packet_buffer *packet = &device->memory->state.tx_packets[id];

  if ((uint16_t)(device->tx_avail_idx - device->tx_used_idx) >= QUEUE_SIZE) {
    errno = ENOSPC;
    fatal("TX virtqueue is full");
  }
  fill_guest_header(device, &packet->header, op, payload_len);
  if (payload_len != 0)
    memset(packet->payload, 0xa5, payload_len);

  tx->desc[id].addr = htole64((uint64_t)(uintptr_t)packet);
  tx->desc[id].len = htole32(sizeof(packet->header) + payload_len);
  tx->desc[id].flags = 0;
  tx->desc[id].next = 0;
  tx->avail.ring[slot] = htole16(id);
  device->tx_avail_idx++;
  store_index(&tx->avail.idx, device->tx_avail_idx);

  if (op == VIRTIO_VSOCK_OP_RW)
    device->metrics.tx_data_packets++;
  else
    device->metrics.tx_control_packets++;
}

static void wait_for_all_tx_used(struct vhost_device *device) {
  struct split_ring *tx = &device->memory->state.tx;

  while (device->tx_used_idx != device->tx_avail_idx) {
    uint16_t used = load_index(&tx->used.idx);

    if (used == device->tx_used_idx) {
      device->metrics.tx_calls +=
          read_event_counter(device->tx_call, "TX call");
      continue;
    }
    if ((uint16_t)(used - device->tx_used_idx) > QUEUE_SIZE) {
      errno = EPROTO;
      fatal("invalid TX used index");
    }
    device->tx_used_idx = used;
  }
}

static void submit_control_packet_and_wait(struct vhost_device *device,
                                           uint16_t op) {
  submit_tx_packet(device, op, 0);
  kick_tx(device, false);
  wait_for_all_tx_used(device);
}

static void update_host_credit(struct vhost_device *device,
                               const struct virtio_vsock_hdr *header) {
  device->host_buf_alloc = le32toh(header->buf_alloc);
  device->host_fwd_cnt = le32toh(header->fwd_cnt);
}

static void validate_host_packet(const struct vhost_device *device,
                                 const struct received_packet *packet) {
  const struct virtio_vsock_hdr *header = &packet->header;

  if (le64toh(header->src_cid) != VMADDR_CID_HOST ||
      le64toh(header->dst_cid) != GUEST_CID ||
      le32toh(header->dst_port) != GUEST_PORT ||
      le16toh(header->type) != VIRTIO_VSOCK_TYPE_STREAM) {
    errno = EPROTO;
    fatal("invalid packet from host");
  }
  if (device->host_port != 0 &&
      le32toh(header->src_port) != device->host_port) {
    errno = EPROTO;
    fatal("packet has unexpected host port: expected=%u actual=%u "
          "op=%u len=%u",
          device->host_port, le32toh(header->src_port), le16toh(header->op),
          packet->payload_len);
  }
}

static void send_credit_update(struct vhost_device *device, bool force) {
  uint32_t forwarded = device->guest_fwd_cnt - device->last_credit_update;

  if (!force && forwarded < CREDIT_UPDATE_THRESHOLD)
    return;
  if ((uint16_t)(device->tx_avail_idx - device->tx_used_idx) >= QUEUE_SIZE)
    wait_for_all_tx_used(device);
  submit_tx_packet(device, VIRTIO_VSOCK_OP_CREDIT_UPDATE, 0);
  kick_tx(device, false);
  device->last_credit_update = device->guest_fwd_cnt;
}

static void handle_control_packet(struct vhost_device *device,
                                  const struct received_packet *packet) {
  uint16_t op = le16toh(packet->header.op);

  if (packet->payload_len != 0) {
    errno = EPROTO;
    fatal("control packet contains a payload");
  }
  device->metrics.rx_control_packets++;
  update_host_credit(device, &packet->header);
  if (op == VIRTIO_VSOCK_OP_CREDIT_REQUEST)
    send_credit_update(device, true);
  else if (op != VIRTIO_VSOCK_OP_CREDIT_UPDATE) {
    errno = EPROTO;
    fatal("unexpected control operation %u", op);
  }
}

static void drain_control_packets(struct vhost_device *device) {
  while (rx_packet_available(device)) {
    struct received_packet packet = receive_packet(device);

    validate_host_packet(device, &packet);
    handle_control_packet(device, &packet);
  }
}

static uint32_t available_host_credit(const struct vhost_device *device) {
  uint32_t used = device->guest_tx_cnt - device->host_fwd_cnt;

  return device->host_buf_alloc > used ? device->host_buf_alloc - used : 0;
}

static void guest_receive(struct vhost_device *device, uint64_t total_bytes) {
  uint64_t received = 0;

  while (received < total_bytes) {
    struct received_packet packet = receive_packet(device);
    uint16_t op;

    validate_host_packet(device, &packet);
    op = le16toh(packet.header.op);
    if (op != VIRTIO_VSOCK_OP_RW) {
      handle_control_packet(device, &packet);
      continue;
    }
    if (packet.payload_len == 0 ||
        packet.payload_len > total_bytes - received) {
      errno = EPROTO;
      fatal("unexpected host-to-guest payload length");
    }
    received += packet.payload_len;
    device->guest_fwd_cnt += packet.payload_len;
    device->metrics.rx_data_packets++;
    send_credit_update(device, false);
  }
  if (device->guest_fwd_cnt != device->last_credit_update)
    send_credit_update(device, true);
  wait_for_all_tx_used(device);
}

static void wait_for_host_credit(struct vhost_device *device) {
  while (available_host_credit(device) == 0) {
    struct received_packet packet = receive_packet(device);

    validate_host_packet(device, &packet);
    handle_control_packet(device, &packet);
  }
}

static void guest_send(struct vhost_device *device, uint64_t total_bytes) {
  uint64_t remaining = total_bytes;

  wait_for_all_tx_used(device);
  while (remaining != 0) {
    uint32_t credit;
    uint16_t submitted = 0;

    drain_control_packets(device);
    wait_for_host_credit(device);
    credit = available_host_credit(device);
    while (remaining != 0 && credit != 0 && submitted < QUEUE_SIZE) {
      uint32_t payload = device->payload_size;

      if (payload > remaining)
        payload = (uint32_t)remaining;
      if (payload > credit)
        payload = credit;
      submit_tx_packet(device, VIRTIO_VSOCK_OP_RW, payload);
      device->guest_tx_cnt += payload;
      remaining -= payload;
      credit -= payload;
      submitted++;
    }
    kick_tx(device, false);
    wait_for_all_tx_used(device);
  }
}

static void configure_vhost_vring(struct vhost_device *device,
                                  unsigned int index, struct split_ring *ring,
                                  int kick_fd, int call_fd) {
  struct vhost_vring_state state = {
      .index = index,
      .num = QUEUE_SIZE,
  };
  struct vhost_vring_addr address = {
      .index = index,
      .desc_user_addr = (uintptr_t)ring->desc,
      .avail_user_addr = (uintptr_t)&ring->avail,
      .used_user_addr = (uintptr_t)&ring->used,
  };
  struct vhost_vring_file file = {
      .index = index,
  };

  checked_ioctl(device->fd, VHOST_SET_VRING_NUM, &state, "VHOST_SET_VRING_NUM");
  state.num = 0;
  checked_ioctl(device->fd, VHOST_SET_VRING_BASE, &state,
                "VHOST_SET_VRING_BASE");
  checked_ioctl(device->fd, VHOST_SET_VRING_ADDR, &address,
                "VHOST_SET_VRING_ADDR");
  file.fd = kick_fd;
  checked_ioctl(device->fd, VHOST_SET_VRING_KICK, &file,
                "VHOST_SET_VRING_KICK");
  file.fd = call_fd;
  checked_ioctl(device->fd, VHOST_SET_VRING_CALL, &file,
                "VHOST_SET_VRING_CALL");
}

static void configure_vhost_user_vring(struct vhost_device *device,
                                       uint32_t index, struct split_ring *ring,
                                       int kick_fd, int call_fd) {
  struct vhost_user_vring_state state = {
      .index = index,
      .num = QUEUE_SIZE,
  };
  struct vhost_user_vring_addr address = {
      .index = index,
      .descriptor = (uintptr_t)ring->desc,
      .used = (uintptr_t)&ring->used,
      .available = (uintptr_t)&ring->avail,
  };
  uint64_t ring_index = index;

  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_NUM, &state,
                          sizeof(state), -1);
  state.num = 0;
  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_BASE, &state,
                          sizeof(state), -1);
  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_ADDR, &address,
                          sizeof(address), -1);
  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_CALL, &ring_index,
                          sizeof(ring_index), call_fd);
  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_KICK, &ring_index,
                          sizeof(ring_index), kick_fd);
  state.num = 1;
  send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_ENABLE, &state,
                          sizeof(state), -1);
}

static int create_eventfd(const char *name) {
  int fd = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);

  if (fd < 0)
    fatal("create %s eventfd", name);
  return fd;
}

static void allocate_guest_memory(struct vhost_device *device) {
  device->memory_fd = memfd_create("vhost-vsock-bench", MFD_CLOEXEC);
  if (device->memory_fd < 0)
    fatal("memfd_create guest memory");
  if (ftruncate(device->memory_fd, sizeof(*device->memory)) < 0)
    fatal("size guest memory");
  device->memory = mmap(NULL, sizeof(*device->memory), PROT_READ | PROT_WRITE,
                        MAP_SHARED, device->memory_fd, 0);
  if (device->memory == MAP_FAILED)
    fatal("map guest memory");
  memset(device->memory, 0, sizeof(*device->memory));
}

static void create_device_eventfds(struct vhost_device *device) {
  device->rx_kick = create_eventfd("RX kick");
  device->rx_call = create_eventfd("RX call");
  device->tx_kick = create_eventfd("TX kick");
  device->tx_call = create_eventfd("TX call");
}

static void start_vhost_device(struct vhost_device *device) {
  uint64_t features = 0;
  int running = 1;

  device->fd = open("/dev/vhost-vsock", O_RDWR | O_CLOEXEC);
  if (device->fd < 0)
    fatal("open /dev/vhost-vsock");

  checked_ioctl(device->fd, VHOST_SET_OWNER, NULL, "VHOST_SET_OWNER");
  checked_ioctl(device->fd, VHOST_SET_FEATURES, &features,
                "VHOST_SET_FEATURES");
  checked_ioctl(device->fd, VHOST_SET_BACKEND_FEATURES, &features,
                "VHOST_SET_BACKEND_FEATURES");

  device->memory_table.memory.nregions = 1;
  device->memory_table.region.guest_phys_addr = (uintptr_t)device->memory;
  device->memory_table.region.memory_size = sizeof(*device->memory);
  device->memory_table.region.userspace_addr = (uintptr_t)device->memory;
  checked_ioctl(device->fd, VHOST_SET_MEM_TABLE, &device->memory_table.memory,
                "VHOST_SET_MEM_TABLE");

  configure_vhost_vring(device, 0, &device->memory->state.rx, device->rx_kick,
                        device->rx_call);
  configure_vhost_vring(device, 1, &device->memory->state.tx, device->tx_kick,
                        device->tx_call);
  checked_ioctl(device->fd, VHOST_VSOCK_SET_GUEST_CID, &(uint64_t){GUEST_CID},
                "VHOST_VSOCK_SET_GUEST_CID");
  publish_initial_rx_buffers(device);
  checked_ioctl(device->fd, VHOST_VSOCK_SET_RUNNING, &running,
                "VHOST_VSOCK_SET_RUNNING");
}

static void start_vhost_user_device(struct vhost_device *device,
                                    const char *socket_path) {
  struct vhost_user_memory_table memory_table = {
      .memory.num_regions = 1,
      .region =
          {
              .guest_phys_addr = (uintptr_t)device->memory,
              .memory_size = sizeof(*device->memory),
              .user_addr = (uintptr_t)device->memory,
          },
  };
  uint64_t offered_features;
  uint64_t offered_protocol_features;
  uint64_t queue_count;
  uint64_t features = VIRTIO_F_VERSION_1_MASK | VHOST_USER_F_PROTOCOL_FEATURES;

  device->fd = connect_unix_socket(socket_path, "vhost-user control");
  send_vhost_user_message(device->fd, VHOST_USER_SET_OWNER, NULL, 0, -1);
  offered_features = get_vhost_user_u64(device->fd, VHOST_USER_GET_FEATURES);
  if ((offered_features & features) != features) {
    errno = ENOTSUP;
    fatal("vhost-user backend lacks required virtio features");
  }
  set_vhost_user_u64(device->fd, VHOST_USER_SET_FEATURES, features);
  offered_protocol_features =
      get_vhost_user_u64(device->fd, VHOST_USER_GET_PROTOCOL_FEATURES);
  if ((offered_protocol_features & VHOST_USER_PROTOCOL_F_MQ) == 0) {
    errno = ENOTSUP;
    fatal("vhost-user backend lacks MQ protocol feature");
  }
  set_vhost_user_u64(device->fd, VHOST_USER_SET_PROTOCOL_FEATURES,
                     VHOST_USER_PROTOCOL_F_MQ);
  queue_count = get_vhost_user_u64(device->fd, VHOST_USER_GET_QUEUE_NUM);
  if (queue_count < 2) {
    errno = ENOTSUP;
    fatal("vhost-user backend provides fewer than two queues");
  }
  send_vhost_user_message(device->fd, VHOST_USER_SET_MEM_TABLE, &memory_table,
                          sizeof(memory_table), device->memory_fd);
  configure_vhost_user_vring(device, 0, &device->memory->state.rx,
                             device->rx_kick, device->rx_call);
  configure_vhost_user_vring(device, 1, &device->memory->state.tx,
                             device->tx_kick, device->tx_call);
  /* Wait until the backend has consumed the preceding one-way setup messages. */
  (void)get_vhost_user_u64(device->fd, VHOST_USER_GET_QUEUE_NUM);
  publish_initial_rx_buffers(device);
}

static void start_device(struct vhost_device *device, enum backend_type backend,
                         uint32_t payload_size, const char *vhost_user_socket) {
  memset(device, 0, sizeof(*device));
  device->backend = backend;
  device->payload_size = payload_size;
  allocate_guest_memory(device);
  create_device_eventfds(device);
  if (backend == BACKEND_VHOST)
    start_vhost_device(device);
  else
    start_vhost_user_device(device, vhost_user_socket);
}

static void complete_guest_connection(struct vhost_device *device) {
  struct received_packet request;

  kick_rx(device, true);
  request = receive_packet(device);
  validate_host_packet(device, &request);
  if (le16toh(request.header.op) != VIRTIO_VSOCK_OP_REQUEST ||
      request.payload_len != 0) {
    errno = EPROTO;
    fatal("expected connection request");
  }
  device->host_port = le32toh(request.header.src_port);
  update_host_credit(device, &request.header);
  device->metrics.rx_control_packets++;
  submit_control_packet_and_wait(device, VIRTIO_VSOCK_OP_RESPONSE);
}

static int connect_vhost_host_socket(struct vhost_device *device) {
  struct sockaddr_vm address = {
      .svm_family = AF_VSOCK,
      .svm_cid = GUEST_CID,
      .svm_port = GUEST_PORT,
  };
  socklen_t error_length;
  int socket_error = 0;
  int socket_fd =
      socket(AF_VSOCK, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);

  if (socket_fd < 0)
    fatal("create AF_VSOCK socket");
  if (connect(socket_fd, (struct sockaddr *)&address, sizeof(address)) < 0 &&
      errno != EINPROGRESS)
    fatal("connect AF_VSOCK socket");
  complete_guest_connection(device);

  wait_for_fd(socket_fd, POLLOUT, "AF_VSOCK connect");
  error_length = sizeof(socket_error);
  if (getsockopt(socket_fd, SOL_SOCKET, SO_ERROR, &socket_error,
                 &error_length) < 0)
    fatal("getsockopt SO_ERROR");
  if (socket_error != 0) {
    errno = socket_error;
    fatal("complete AF_VSOCK connect");
  }
  return socket_fd;
}

static void read_host_line(int fd, char *line, size_t capacity) {
  size_t length = 0;

  while (length + 1 < capacity) {
    ssize_t received = read(fd, &line[length], 1);

    if (received == 1) {
      if (line[length++] == '\n') {
        line[length] = '\0';
        return;
      }
      continue;
    }
    if (received < 0 && errno == EINTR)
      continue;
    if (received < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      wait_for_fd(fd, POLLIN, "vhost-user host handshake");
      continue;
    }
    if (received == 0)
      errno = ECONNRESET;
    fatal("read vhost-user host handshake");
  }
  errno = EMSGSIZE;
  fatal("vhost-user host handshake is too long");
}

static int connect_vhost_user_host_socket(struct vhost_device *device,
                                          const char *socket_path) {
  char response[64];
  char request[32];
  int socket_fd = connect_unix_socket(socket_path, "vhost-user host");
  int request_length;
  unsigned int peer_port;
  char trailing;

  set_nonblocking(socket_fd, "vhost-user host socket");
  request_length =
      snprintf(request, sizeof(request), "CONNECT %u\n", GUEST_PORT);
  if (request_length < 0 || (size_t)request_length >= sizeof(request)) {
    errno = EOVERFLOW;
    fatal("format vhost-user host request");
  }
  write_full(socket_fd, request, (size_t)request_length,
             "vhost-user host handshake");
  complete_guest_connection(device);
  read_host_line(socket_fd, response, sizeof(response));
  if (sscanf(response, "OK %u%c", &peer_port, &trailing) != 2 ||
      trailing != '\n') {
    errno = EPROTO;
    fatal("invalid vhost-user host response '%s'", response);
  }
  return socket_fd;
}

static int connect_host_socket(struct vhost_device *device,
                               const char *vhost_user_host_socket) {
  if (device->backend == BACKEND_VHOST)
    return connect_vhost_host_socket(device);
  return connect_vhost_user_host_socket(device, vhost_user_host_socket);
}

static void disconnect_host_socket(struct vhost_device *device, int socket_fd) {
  if (shutdown(socket_fd, SHUT_RDWR) < 0)
    fatal("shutdown host socket");

  for (;;) {
    struct received_packet packet;
    uint16_t op;

    packet = receive_packet(device);
    validate_host_packet(device, &packet);
    op = le16toh(packet.header.op);
    if (op == VIRTIO_VSOCK_OP_SHUTDOWN) {
      if (packet.payload_len != 0) {
        errno = EPROTO;
        fatal("shutdown packet contains a payload");
      }
      device->metrics.rx_control_packets++;
      break;
    }
    handle_control_packet(device, &packet);
  }

  submit_control_packet_and_wait(device, VIRTIO_VSOCK_OP_RST);
  if (close(socket_fd) < 0)
    fatal("close host socket");
}

static void stop_device(struct vhost_device *device) {
  if (device->backend == BACKEND_VHOST) {
    int running = 0;

    checked_ioctl(device->fd, VHOST_VSOCK_SET_RUNNING, &running,
                  "stop vhost-vsock");
  } else {
    for (uint32_t index = 0; index < 2; index++) {
      struct vhost_user_vring_state state = {
          .index = index,
      };

      send_vhost_user_message(device->fd, VHOST_USER_SET_VRING_ENABLE, &state,
                              sizeof(state), -1);
    }
  }

  if (close(device->rx_kick) < 0 || close(device->rx_call) < 0 ||
      close(device->tx_kick) < 0 || close(device->tx_call) < 0 ||
      close(device->fd) < 0)
    fatal("close vhost-vsock descriptor");
  if (munmap(device->memory, sizeof(*device->memory)) < 0)
    fatal("unmap guest memory");
  if (close(device->memory_fd) < 0)
    fatal("close guest memory");
}

static void host_send_all(struct host_io *io) {
  uint64_t remaining = io->total_bytes;

  while (remaining != 0) {
    size_t length = io->buffer_size;
    ssize_t sent;

    if (length > remaining)
      length = (size_t)remaining;
    sent = send(io->socket_fd, io->buffer, length, 0);
    if (sent > 0) {
      remaining -= (uint64_t)sent;
      io->calls++;
      continue;
    }
    if (sent < 0 && errno == EINTR)
      continue;
    if (sent < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      wait_for_fd(io->socket_fd, POLLOUT, "host send");
      continue;
    }
    if (sent == 0)
      errno = EPIPE;
    fatal("host send");
  }
}

static void host_receive_all(struct host_io *io) {
  uint64_t remaining = io->total_bytes;

  while (remaining != 0) {
    size_t length = io->buffer_size;
    ssize_t received;

    if (length > remaining)
      length = (size_t)remaining;
    received = recv(io->socket_fd, io->buffer, length, 0);
    if (received > 0) {
      remaining -= (uint64_t)received;
      io->calls++;
      continue;
    }
    if (received < 0 && errno == EINTR)
      continue;
    if (received < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      wait_for_fd(io->socket_fd, POLLIN, "host receive");
      continue;
    }
    if (received == 0)
      errno = ECONNRESET;
    fatal("host receive");
  }
}

static void *host_io_thread(void *argument) {
  struct host_io *io = argument;

  (void)read_event_counter(io->start_fd, "benchmark start");
  atomic_store_explicit(&io->started_ns, monotonic_nanoseconds(),
                        memory_order_release);
  if (io->direction == DIRECTION_H2G)
    host_send_all(io);
  else
    host_receive_all(io);
  atomic_store_explicit(&io->completed_ns, monotonic_nanoseconds(),
                        memory_order_release);
  return NULL;
}

static void reset_metrics(struct vhost_device *device) {
  (void)drain_event_counter(device->rx_call, "RX call");
  (void)drain_event_counter(device->tx_call, "TX call");
  memset(&device->metrics, 0, sizeof(device->metrics));
}

static double run_transfer(struct vhost_device *device, int socket_fd,
                           enum direction direction, uint64_t total_bytes,
                           uint64_t *host_calls) {
  struct host_io host_io = {
      .socket_fd = socket_fd,
      .start_fd = create_eventfd("benchmark start"),
      .direction = direction,
      .buffer_size = device->payload_size,
      .total_bytes = total_bytes,
  };
  pthread_t thread;
  uint64_t start;
  uint64_t guest_completed_ns;
  uint64_t host_completed_ns;

  host_io.buffer = malloc(host_io.buffer_size);
  if (host_io.buffer == NULL)
    fatal("allocate host I/O buffer");
  memset(host_io.buffer, 0x5a, host_io.buffer_size);
  int thread_error = pthread_create(&thread, NULL, host_io_thread, &host_io);
  if (thread_error != 0) {
    errno = thread_error;
    fatal("pthread_create");
  }

  write_event_counter(host_io.start_fd, "benchmark start");
  while ((start = atomic_load_explicit(&host_io.started_ns,
                                       memory_order_acquire)) == 0)
    sched_yield();
  if (direction == DIRECTION_H2G)
    guest_receive(device, total_bytes);
  else
    guest_send(device, total_bytes);
  guest_completed_ns = monotonic_nanoseconds();
  thread_error = pthread_join(thread, NULL);
  if (thread_error != 0) {
    errno = thread_error;
    fatal("pthread_join");
  }
  host_completed_ns =
      atomic_load_explicit(&host_io.completed_ns, memory_order_acquire);
  *host_calls = host_io.calls;
  free(host_io.buffer);
  if (close(host_io.start_fd) < 0)
    fatal("close benchmark start eventfd");
  if (host_completed_ns > guest_completed_ns)
    guest_completed_ns = host_completed_ns;
  return (double)(guest_completed_ns - start) / 1000000000.0;
}

static void print_usage(const char *program) {
  fprintf(stderr,
          "Usage: %s --direction <h2g|g2h> [OPTIONS]\n"
          "  --backend <vhost|vhost-user>  Backend (default: vhost)\n"
          "  --bytes <size>         Timed transfer size (default: 16M)\n"
          "  --buf-size <size>      Per-operation payload, up to 64K\n"
          "  --warmup-bytes <size>  Untimed warmup size (default: 1M)\n"
          "  --vhost-user-socket <path>  vhost-user control socket\n"
          "  --host-socket <path>   vhost-user host endpoint socket\n",
          program);
}

int main(int argc, char **argv) {
  static const struct option options[] = {
      {"backend", required_argument, NULL, 'B'},
      {"direction", required_argument, NULL, 'd'},
      {"bytes", required_argument, NULL, 'b'},
      {"buf-size", required_argument, NULL, 's'},
      {"warmup-bytes", required_argument, NULL, 'w'},
      {"vhost-user-socket", required_argument, NULL, 'u'},
      {"host-socket", required_argument, NULL, 'o'},
      {"help", no_argument, NULL, 'h'},
      {NULL, 0, NULL, 0},
  };
  struct vhost_device device;
  enum backend_type backend = BACKEND_VHOST;
  enum direction direction = DIRECTION_H2G;
  const char *backend_text = "vhost";
  const char *direction_name = NULL;
  const char *vhost_user_socket = NULL;
  const char *vhost_user_host_socket = NULL;
  uint64_t total_bytes = DEFAULT_TOTAL_BYTES;
  uint64_t warmup_bytes = DEFAULT_WARMUP_BYTES;
  uint64_t buffer_size = 0;
  uint64_t host_calls = 0;
  double seconds;
  double mbits_per_second;
  int socket_fd;
  int option;

  if (signal(SIGPIPE, SIG_IGN) == SIG_ERR)
    fatal("ignore SIGPIPE");

  while ((option = getopt_long(argc, argv, "B:d:b:s:w:u:o:h", options, NULL)) !=
         -1) {
    switch (option) {
    case 'B':
      backend_text = optarg;
      break;
    case 'd':
      direction_name = optarg;
      break;
    case 'b':
      total_bytes = parse_size(optarg, "byte count");
      break;
    case 's':
      buffer_size = parse_size(optarg, "buffer size");
      break;
    case 'w':
      warmup_bytes = parse_size(optarg, "warmup byte count");
      break;
    case 'u':
      vhost_user_socket = optarg;
      break;
    case 'o':
      vhost_user_host_socket = optarg;
      break;
    case 'h':
      print_usage(argv[0]);
      return EXIT_SUCCESS;
    default:
      print_usage(argv[0]);
      return EXIT_FAILURE;
    }
  }
  if (optind != argc || direction_name == NULL || buffer_size == 0 ||
      buffer_size > MAX_PAYLOAD_SIZE || total_bytes == 0) {
    print_usage(argv[0]);
    return EXIT_FAILURE;
  }
  if (strcmp(backend_text, "vhost") == 0)
    backend = BACKEND_VHOST;
  else if (strcmp(backend_text, "vhost-user") == 0)
    backend = BACKEND_VHOST_USER;
  else {
    fprintf(stderr, "invalid backend '%s'\n", backend_text);
    return EXIT_FAILURE;
  }
  if (backend == BACKEND_VHOST_USER &&
      (vhost_user_socket == NULL || vhost_user_host_socket == NULL)) {
    fprintf(stderr,
            "vhost-user requires --vhost-user-socket and --host-socket\n");
    return EXIT_FAILURE;
  }
  if (strcmp(direction_name, "h2g") == 0)
    direction = DIRECTION_H2G;
  else if (strcmp(direction_name, "g2h") == 0)
    direction = DIRECTION_G2H;
  else {
    fprintf(stderr, "invalid direction '%s'\n", direction_name);
    return EXIT_FAILURE;
  }

  if (warmup_bytes != 0) {
    start_device(&device, backend, (uint32_t)buffer_size, vhost_user_socket);
    socket_fd = connect_host_socket(&device, vhost_user_host_socket);
    (void)run_transfer(&device, socket_fd, direction, warmup_bytes,
                       &host_calls);
    drain_control_packets(&device);
    disconnect_host_socket(&device, socket_fd);
    stop_device(&device);
  }
  start_device(&device, backend, (uint32_t)buffer_size, vhost_user_socket);
  socket_fd = connect_host_socket(&device, vhost_user_host_socket);
  reset_metrics(&device);
  seconds =
      run_transfer(&device, socket_fd, direction, total_bytes, &host_calls);
  drain_control_packets(&device);
  device.metrics.rx_calls += drain_event_counter(device.rx_call, "RX call");
  device.metrics.tx_calls += drain_event_counter(device.tx_call, "TX call");
  mbits_per_second = (double)total_bytes * 8.0 / seconds / 1000000.0;

  printf("VHOST_VSOCK_RESULT %s %s %" PRIu64 " %" PRIu64 " %.9f %.3f\n",
         backend_name(backend), direction_name, buffer_size, total_bytes,
         seconds, mbits_per_second);
  printf("VHOST_VSOCK_PROFILE %s host_calls=%" PRIu64
         " rx_data_packets=%" PRIu64 " tx_data_packets=%" PRIu64
         " rx_control_packets=%" PRIu64 " tx_control_packets=%" PRIu64
         " rx_kicks=%" PRIu64 " tx_kicks=%" PRIu64 " rx_calls=%" PRIu64
         " tx_calls=%" PRIu64 "\n",
         backend_name(backend), host_calls, device.metrics.rx_data_packets,
         device.metrics.tx_data_packets, device.metrics.rx_control_packets,
         device.metrics.tx_control_packets, device.metrics.rx_kicks,
         device.metrics.tx_kicks, device.metrics.rx_calls,
         device.metrics.tx_calls);

  disconnect_host_socket(&device, socket_fd);
  stop_device(&device);
  return EXIT_SUCCESS;
}
