// SPDX-License-Identifier: MPL-2.0

/* Tableless DM identity management and copied ioctl parameter boundaries. */
#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/dm-ioctl.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../common/test.h"

#define DM_CONTROL_PATH "/dev/mapper/control"
#define DM_IOCTL_ENVELOPE_SIZE 312
#define DM_BUFFER_SIZE 512
#define DM_IOCTL_FIXED_SIZE 305

union dm_ioctl_buffer {
	struct dm_ioctl align;
	unsigned char bytes[DM_BUFFER_SIZE];
};

/* Pin requests and response expectations to Linux 6.6 (5078a3a23a1e),
 * independently of the build environment's Linux header version.
 */
static const uint32_t kernel_version[3] = { 4, 48, 0 };

static inline void init_ioctl(struct dm_ioctl *io)
{
	memset(io, 0, sizeof(*io));
	memcpy(io->version, kernel_version, sizeof(io->version));
	io->data_size = sizeof(*io);
	io->data_start = sizeof(*io);
}

static inline void init_named_ioctl(struct dm_ioctl *io, const char *name)
{
	init_ioctl(io);
	snprintf(io->name, sizeof(io->name), "%s", name);
}

static inline size_t align_to_u64(size_t value)
{
	return (value + sizeof(uint64_t) - 1) & ~(sizeof(uint64_t) - 1);
}

/* Raw readers avoid alignment assumptions for guard-page ioctl arguments. */
static inline uint32_t raw_u32(const unsigned char *bytes)
{
	uint32_t value;
	memcpy(&value, bytes, sizeof(value));
	return value;
}

static inline uint64_t raw_u64(const unsigned char *bytes)
{
	uint64_t value;
	memcpy(&value, bytes, sizeof(value));
	return value;
}

static int bytes_are(const unsigned char *bytes, size_t len,
		     unsigned char value)
{
	for (size_t i = 0; i < len; i++) {
		if (bytes[i] != value)
			return 0;
	}
	return 1;
}

FN_TEST(device_mapper_tableless_status_remove_and_reuse)
{
	char name[DM_NAME_LEN], path[256];
	struct dm_ioctl io;
	struct stat st;
	uint64_t dev;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	snprintf(name, sizeof(name), "dm-tableless-%ld", (long)getpid());
	init_named_ioctl(&io, name);
	io.flags = DM_READONLY_FLAG;
	CHECK(ioctl(fd, DM_DEV_CREATE, &io));
	dev = io.dev;
	TEST_RES(io.flags & (DM_READONLY_FLAG | DM_ACTIVE_PRESENT_FLAG |
			     DM_INACTIVE_PRESENT_FLAG | DM_SUSPEND_FLAG),
		 _ret == 0);
	TEST_RES(io.target_count, _ret == 0);
	TEST_RES(io.open_count, _ret == 0);
	TEST_RES(io.event_nr, _ret == 0);

	/* Identity creation does not publish a data-plane node in this PR. */
	snprintf(path, sizeof(path), "/dev/dm-%u", minor(dev));
	TEST(stat(path, &st), ENOENT, _ret == -1);
	snprintf(path, sizeof(path), "/dev/mapper/%s", name);
	TEST(stat(path, &st), ENOENT, _ret == -1);
	for (int inactive = 0; inactive < 2; inactive++) {
		init_ioctl(&io);
		io.dev = dev | (UINT64_C(1) << 32);
		io.flags = inactive ? DM_QUERY_INACTIVE_TABLE_FLAG : 0;
		TEST_SUCC(ioctl(fd, DM_DEV_STATUS, &io));
		TEST_RES(io.dev, _ret == dev);
		TEST_RES(io.target_count, _ret == 0);
		TEST_RES(io.open_count, _ret == 0);
		TEST_RES(io.event_nr, _ret == 0);
	}

	init_named_ioctl(&io, name);
	io.flags = DM_DEFERRED_REMOVE;
	TEST(ioctl(fd, DM_DEV_REMOVE, &io), EOPNOTSUPP, _ret == -1);
	init_named_ioctl(&io, name);
	TEST_SUCC(ioctl(fd, DM_DEV_STATUS, &io));
	init_named_ioctl(&io, name);
	io.event_nr = 0x12345678;
	io.target_count = 17;
	io.open_count = 9;
	io.flags = DM_INACTIVE_PRESENT_FLAG;
	TEST_SUCC(ioctl(fd, DM_DEV_REMOVE, &io));
	TEST_RES(io.dev, _ret == 0);
	TEST_RES(io.event_nr, _ret == 0x12345678);
	TEST_RES(io.target_count, _ret == 17);
	TEST_RES(io.open_count, _ret == 9);
	TEST_RES(io.flags & DM_INACTIVE_PRESENT_FLAG, _ret == 0);
	init_named_ioctl(&io, name);
	TEST(ioctl(fd, DM_DEV_STATUS, &io), ENXIO, _ret == -1);
	init_named_ioctl(&io, name);
	TEST(ioctl(fd, DM_DEV_REMOVE, &io), ENXIO, _ret == -1);

	/* The number is reusable as soon as successful REMOVE returns. */
	init_named_ioctl(&io, name);
	io.flags = DM_PERSISTENT_DEV_FLAG;
	io.dev = dev;
	TEST_SUCC(ioctl(fd, DM_DEV_CREATE, &io));
	TEST_RES(io.dev, _ret == dev);
	init_named_ioctl(&io, name);
	TEST_SUCC(ioctl(fd, DM_DEV_REMOVE, &io));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_secure_failure_and_copyout_commit)
{
	union dm_ioctl_buffer input, before;
	struct dm_ioctl io;
	char name[DM_NAME_LEN];
	long page_size = CHECK_WITH(sysconf(_SC_PAGESIZE), _ret > 0);
	unsigned char *pages =
		CHECK_WITH(mmap(NULL, page_size * 2, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	unsigned char *arg = pages + page_size - 64;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	for (int secure = 0; secure < 2; secure++) {
		memset(input.bytes, 0xa5, sizeof(input.bytes));
		init_ioctl(&input.align);
		input.align.version[1] = 0;
		input.align.data_size = sizeof(input.bytes);
		input.align.flags = secure ? DM_SECURE_DATA_FLAG : 0;
		memcpy(&before, &input, sizeof(before));
		/* CREATE rejects an empty name after clearing only the fixed prefix. */
		TEST(ioctl(fd, DM_DEV_CREATE, &input), EINVAL, _ret == -1);
		if (secure) {
			TEST_RES(bytes_are(input.bytes, DM_IOCTL_FIXED_SIZE, 0),
				 _ret == 1);
		} else {
			TEST_RES(memcmp(input.align.version, kernel_version,
					sizeof(kernel_version)),
				 _ret == 0);
			TEST_RES(memcmp(input.bytes + 12, before.bytes + 12,
					DM_IOCTL_FIXED_SIZE - 12),
				 _ret == 0);
		}
		TEST_RES(memcmp(input.bytes + DM_IOCTL_FIXED_SIZE,
				before.bytes + DM_IOCTL_FIXED_SIZE,
				sizeof(input.bytes) - DM_IOCTL_FIXED_SIZE),
			 _ret == 0);
	}

	/* A response-copy fault does not roll back a successfully created identity. */
	snprintf(name, sizeof(name), "dm-copyout-%ld", (long)getpid());
	init_named_ioctl(&io, name);
	memcpy(arg, &io, sizeof(io));
	CHECK(mprotect(pages + page_size, page_size, PROT_READ));
	TEST(ioctl(fd, DM_DEV_CREATE, arg), EFAULT, _ret == -1);
	init_named_ioctl(&io, name);
	TEST_SUCC(ioctl(fd, DM_DEV_STATUS, &io));

	/* Removal also remains committed when its response copy faults. */
	CHECK(mprotect(pages + page_size, page_size, PROT_READ | PROT_WRITE));
	init_named_ioctl(&io, name);
	memcpy(arg, &io, sizeof(io));
	CHECK(mprotect(pages + page_size, page_size, PROT_READ));
	TEST(ioctl(fd, DM_DEV_REMOVE, arg), EFAULT, _ret == -1);
	init_named_ioctl(&io, name);
	TEST(ioctl(fd, DM_DEV_STATUS, &io), ENXIO, _ret == -1);
	TEST_SUCC(munmap(pages, page_size * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_opaque_uuid)
{
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));
	struct dm_ioctl io;
	char name[DM_NAME_LEN];
	char other[DM_NAME_LEN];
	char uuid[DM_UUID_LEN];

	snprintf(name, sizeof(name), "dm-bytes-%ld", (long)getpid());
	snprintf(other, sizeof(other), "dm-bytes-other-%ld", (long)getpid());
	snprintf(uuid, sizeof(uuid), "opaque-%ld-\xff\x80", (long)getpid());
	init_named_ioctl(&io, name);
	memcpy(io.uuid, uuid, strlen(uuid) + 1);
	TEST_SUCC(ioctl(fd, DM_DEV_CREATE, &io));
	TEST_RES(memcmp(io.uuid, uuid, strlen(uuid) + 1), _ret == 0);

	init_ioctl(&io);
	memcpy(io.uuid, uuid, strlen(uuid) + 1);
	TEST_SUCC(ioctl(fd, DM_DEV_STATUS, &io));
	TEST_RES(strcmp(io.name, name), _ret == 0);
	TEST_RES(memcmp(io.uuid, uuid, strlen(uuid) + 1), _ret == 0);
	init_named_ioctl(&io, other);
	memcpy(io.uuid, uuid, strlen(uuid) + 1);
	TEST(ioctl(fd, DM_DEV_CREATE, &io), EBUSY, _ret == -1);
	init_ioctl(&io);
	memcpy(io.uuid, uuid, strlen(uuid) + 1);
	TEST_SUCC(ioctl(fd, DM_DEV_REMOVE, &io));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_reports_version_before_parameter_errors)
{
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	for (int i = 0; i < 3; i++) {
		struct dm_ioctl io;

		init_ioctl(&io);
		io.version[1] = 0;
		io.version[2] = UINT32_MAX;
		if (i == 0)
			io.data_size = 0;
		else if (i == 1)
			io.flags = 1U << 31;
		else
			memset(io.name, 'x', sizeof(io.name));

		/* Parameter rejection must not hide the earlier ABI handshake. */
		TEST(ioctl(fd, DM_DEV_STATUS, &io), i == 0 ? EINVAL : ENXIO,
		     _ret == -1);
		TEST_RES(memcmp(io.version, kernel_version, sizeof(io.version)),
			 _ret == 0);
	}
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_fixed_prefix_and_no_params_guard_pages)
{
	const size_t sizes[] = { 305, 311, 312, 1024 * 1024,
				 1024 * 1024 * 1024 };
	long page_size = CHECK_WITH(sysconf(_SC_PAGESIZE), _ret > 0);
	unsigned char *pages =
		CHECK_WITH(mmap(NULL, page_size * 2, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	char name[DM_NAME_LEN];
	struct dm_ioctl io;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	TEST_RES(offsetof(struct dm_ioctl, data), _ret == DM_IOCTL_FIXED_SIZE);
	snprintf(name, sizeof(name), "dm-prefix-%ld", (long)getpid());
	CHECK(mprotect(pages + page_size, page_size, PROT_NONE));
	init_named_ioctl(&io, name);
	CHECK(ioctl(fd, DM_DEV_CREATE, &io));
	for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		size_t accessible = sizes[i] > 312 ? 305 : sizes[i];
		unsigned char *arg = pages + page_size - accessible;

		for (int secure = 0; secure < 2; secure++) {
			init_named_ioctl(&io, name);
			io.data_size = sizes[i];
			io.data_start = UINT32_MAX;
			io.flags = secure ? DM_SECURE_DATA_FLAG : 0;
			memset(arg, 0xa5, accessible);
			memcpy(arg, &io, DM_IOCTL_FIXED_SIZE);
			/* Direction/size bits do not change the command's copy contract. */
			TEST_RES(ioctl(fd, _IO(0xfd, DM_DEV_STATUS_CMD), arg),
				 _ret == 0);
			TEST_RES(raw_u32(arg + 12),
				 _ret == DM_IOCTL_FIXED_SIZE);
			TEST_RES(raw_u32(arg + 16), _ret == UINT32_MAX);
			TEST_RES(raw_u32(arg + 28) & DM_SECURE_DATA_FLAG,
				 _ret == 0);
			TEST_RES(bytes_are(arg + 305, accessible - 305, 0xa5),
				 _ret == 1);
		}
	}
	init_named_ioctl(&io, name);
	TEST_RES(ioctl(fd, DM_DEV_REMOVE, &io), _ret == 0);
	TEST_SUCC(munmap(pages, page_size * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_secure_fault_prevents_dispatch)
{
	long page_size = CHECK_WITH(sysconf(_SC_PAGESIZE), _ret > 0);
	unsigned char *pages =
		CHECK_WITH(mmap(NULL, page_size * 2, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	unsigned char *arg;
	struct dm_ioctl io;
	unsigned char before[DM_IOCTL_FIXED_SIZE];
	char name[DM_NAME_LEN];
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	snprintf(name, sizeof(name), "dm-clear-fault-%ld", (long)getpid());
	/* Version and the fixed prefix are writable, but payload copyin faults. */
	arg = pages + page_size - 312;
	init_named_ioctl(&io, name);
	io.data_size = 384;
	io.flags = DM_SECURE_DATA_FLAG;
	memcpy(arg, &io, sizeof(io));
	memcpy(before, arg, sizeof(before));
	CHECK(mprotect(pages + page_size, page_size, PROT_NONE));
	TEST(ioctl(fd, DM_LIST_DEVICES, arg), EFAULT, _ret == -1);
	TEST_RES(memcmp(arg + 12, before + 12, sizeof(before) - 12), _ret == 0);

	/* A readable but read-only header tail fails secure clear before create. */
	CHECK(mprotect(pages + page_size, page_size, PROT_READ | PROT_WRITE));
	arg = pages + page_size - 64;
	init_named_ioctl(&io, name);
	io.flags = DM_SECURE_DATA_FLAG;
	memcpy(arg, &io, sizeof(io));
	memcpy(before, arg, sizeof(before));
	CHECK(mprotect(pages + page_size, page_size, PROT_READ));
	TEST(ioctl(fd, DM_DEV_CREATE, arg), EFAULT, _ret == -1);
	TEST_RES(memcmp(arg + 64, before + 64, sizeof(before) - 64), _ret == 0);
	init_named_ioctl(&io, name);
	TEST(ioctl(fd, DM_DEV_STATUS, &io), ENXIO, _ret == -1);
	TEST_SUCC(munmap(pages, page_size * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_selectors_and_query_flags)
{
	char name[DM_NAME_LEN], uuid[DM_UUID_LEN];
	struct dm_ioctl io;
	uint64_t dev;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));
	const uint32_t preserved = DM_SKIP_BDGET_FLAG | DM_SKIP_LOCKFS_FLAG |
				   DM_NOFLUSH_FLAG | DM_STATUS_TABLE_FLAG |
				   (1U << 31);
	const uint32_t cleared = DM_BUFFER_FULL_FLAG |
				 DM_UEVENT_GENERATED_FLAG |
				 DM_SECURE_DATA_FLAG | DM_DATA_OUT_FLAG |
				 DM_ACTIVE_PRESENT_FLAG |
				 DM_INACTIVE_PRESENT_FLAG | DM_SUSPEND_FLAG;

	snprintf(name, sizeof(name), "dm-select-%ld", (long)getpid());
	snprintf(uuid, sizeof(uuid), "dm-select-uuid-%ld", (long)getpid());
	init_named_ioctl(&io, name);
	snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
	CHECK(ioctl(fd, DM_DEV_CREATE, &io));
	dev = io.dev;
	for (unsigned mask = 0; mask < 8; mask++) {
		int expected = mask == 3 || mask == 7 ? EINVAL : ENXIO;
		init_ioctl(&io);
		if (mask & 1)
			snprintf(io.name, sizeof(io.name), "%s", name);
		if (mask & 2)
			snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
		if (mask & 4)
			io.dev = dev;
		if (mask == 1 || mask == 2 || mask == 4) {
			TEST_RES(ioctl(fd, DM_DEV_STATUS, &io), _ret == 0);
			TEST_RES(io.dev, _ret == dev);
		} else {
			TEST(ioctl(fd, DM_DEV_STATUS, &io), expected,
			     _ret == -1);
		}
	}
	init_named_ioctl(&io, name);
	io.flags = preserved | cleared;
	TEST_RES(ioctl(fd, DM_DEV_STATUS, &io), _ret == 0);
	TEST_RES(io.flags & preserved, _ret == preserved);
	TEST_RES(io.flags & cleared, _ret == 0);
	TEST_RES(io.flags & (1U << 2), _ret == 0);
	init_named_ioctl(&io, name);
	TEST_RES(ioctl(fd, DM_DEV_REMOVE, &io), _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_name_list_raw_layout_and_empty_copyout)
{
	char name[DM_NAME_LEN], uuid[DM_UUID_LEN];
	struct dm_ioctl io;
	union dm_ioctl_buffer output;
	uint64_t dev;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));
	size_t extension, needed;

	snprintf(name, sizeof(name), "dm-list-%ld", (long)getpid());
	snprintf(uuid, sizeof(uuid), "dm-list-uuid-%ld", (long)getpid());
	init_named_ioctl(&io, name);
	snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
	CHECK(ioctl(fd, DM_DEV_CREATE, &io));
	dev = io.dev;
	extension = align_to_u64(12 + strlen(name) + 1);
	for (int with_uuid = 0; with_uuid < 2; with_uuid++) {
		needed = align_to_u64(extension + 8 +
				      (with_uuid ? strlen(uuid) + 1 : 0));
		memset(output.bytes, 0xa5, sizeof(output.bytes));
		init_named_ioctl(&output.align, name);
		snprintf(output.align.uuid, sizeof(output.align.uuid), "%s",
			 uuid);
		output.align.data_size = sizeof(output.bytes);
		output.align.flags = with_uuid ? DM_UUID_FLAG : 0;
		TEST_RES(ioctl(fd, DM_LIST_DEVICES, output.bytes), _ret == 0);
		TEST_RES(output.align.data_start, _ret == 312);
		TEST_RES(output.align.data_size, _ret == 312 + needed);
		TEST_RES(raw_u64(output.bytes + 312), _ret == dev);
		TEST_RES(raw_u32(output.bytes + 320), _ret == 0);
		TEST_RES(memcmp(output.bytes + 324, name, strlen(name) + 1),
			 _ret == 0);
		TEST_RES(raw_u32(output.bytes + 312 + extension), _ret == 0);
		TEST_RES(raw_u32(output.bytes + 316 + extension),
			 _ret == (unsigned)with_uuid);
		if (with_uuid)
			TEST_RES(memcmp(output.bytes + 320 + extension, uuid,
					strlen(uuid) + 1),
				 _ret == 0);
		TEST_RES(bytes_are(output.bytes + 312 + needed,
				   sizeof(output.bytes) - 312 - needed, 0xa5),
			 _ret == 1);

		memset(output.bytes, 0xa5, sizeof(output.bytes));
		init_named_ioctl(&output.align, name);
		output.align.flags = with_uuid ? DM_UUID_FLAG : 0;
		output.align.data_size = 312 + needed - 1;
		TEST_RES(ioctl(fd, DM_LIST_DEVICES, output.bytes), _ret == 0);
		TEST_RES(output.align.flags & DM_BUFFER_FULL_FLAG, _ret != 0);
		TEST_RES(output.align.data_size, _ret == 305);
		TEST_RES(bytes_are(output.bytes + 312,
				   sizeof(output.bytes) - 312, 0xa5),
			 _ret == 1);
	}

	memset(output.bytes, 0xa5, sizeof(output.bytes));
	init_named_ioctl(&output.align, name);
	snprintf(output.align.uuid, sizeof(output.align.uuid), "no-such-uuid");
	output.align.data_size = sizeof(output.bytes);
	TEST_RES(ioctl(fd, DM_LIST_DEVICES, output.bytes), _ret == 0);
	TEST_RES(output.align.data_size, _ret == 312);
	/* Linux prepares a sentinel internally but does not include it in copyout. */
	TEST_RES(bytes_are(output.bytes + 312, sizeof(output.bytes) - 312,
			   0xa5),
		 _ret == 1);
	init_named_ioctl(&io, name);
	TEST_RES(ioctl(fd, DM_DEV_REMOVE, &io), _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_persistent_minor_error_precedes_identity_collision)
{
	char name[DM_NAME_LEN], other[DM_NAME_LEN], uuid[DM_UUID_LEN];
	struct dm_ioctl io;
	uint64_t dev;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	snprintf(name, sizeof(name), "dm-create-order-%ld", (long)getpid());
	snprintf(other, sizeof(other), "dm-create-other-%ld", (long)getpid());
	snprintf(uuid, sizeof(uuid), "dm-create-uuid-%ld", (long)getpid());
	init_named_ioctl(&io, name);
	snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
	if (TEST_RES(ioctl(fd, DM_DEV_CREATE, &io), _ret == 0) < 0)
		goto close_control;
	dev = io.dev;
	for (int collision = 0; collision < 3; collision++) {
		init_named_ioctl(&io, collision == 1 ? name : other);
		if (collision == 2)
			snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
		io.flags = DM_PERSISTENT_DEV_FLAG;
		io.dev = dev | (UINT64_C(1) << 32);
		TEST(ioctl(fd, DM_DEV_CREATE, &io), ENXIO, _ret == -1);
	}
	/* Linux dm_hash_insert reports EBUSY for either identity collision. */
	init_named_ioctl(&io, name);
	TEST(ioctl(fd, DM_DEV_CREATE, &io), EBUSY, _ret == -1);
	init_named_ioctl(&io, other);
	snprintf(io.uuid, sizeof(io.uuid), "%s", uuid);
	TEST(ioctl(fd, DM_DEV_CREATE, &io), EBUSY, _ret == -1);
	init_named_ioctl(&io, name);
	TEST_RES(ioctl(fd, DM_DEV_STATUS, &io), _ret == 0);
	TEST_RES(io.dev, _ret == dev);
	TEST_RES(memcmp(io.uuid, uuid, strlen(uuid) + 1), _ret == 0);
	init_named_ioctl(&io, other);
	TEST(ioctl(fd, DM_DEV_STATUS, &io), ENXIO, _ret == -1);
	init_ioctl(&io);
	io.dev = dev;
	TEST_RES(ioctl(fd, DM_DEV_REMOVE, &io), _ret == 0);
close_control:
	TEST_RES(close(fd), _ret == 0);
}
END_TEST()
