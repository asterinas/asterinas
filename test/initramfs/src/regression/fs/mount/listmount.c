// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/mount.h>
#include <linux/stat.h>
#include <sched.h>
#include <stdint.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define MNT_UNIQUE_ID_MIN ((1ULL << 31) + 1)

#define LM_PARENT "/tmp/lm_parent"
#define LM_CHILD "/tmp/lm_parent/child"

FN_SETUP(setup)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mkdir(LM_PARENT, 0755));
	CHECK(mount("tmpfs", LM_PARENT, "tmpfs", 0, NULL));
	CHECK(mkdir(LM_CHILD, 0755));
	CHECK(mount("tmpfs", LM_CHILD, "tmpfs", 0, NULL));
}
END_SETUP()

static uint64_t mount_unique_id(const char *path)
{
	struct statx stx;
	int r = statx(AT_FDCWD, path, 0, STATX_MNT_ID_UNIQUE, &stx);
	if (r < 0) {
		return 0;
	}
	return stx.stx_mnt_id;
}

FN_TEST(listmount_lsmt_root_lists_root_descendants)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[16];
	TEST_RES(syscall(SYS_listmount, &req, ids, 16, 0), _ret >= 1);
}
END_TEST()

FN_TEST(listmount_lists_descendants_of_specific_parent)
{
	uint64_t parent_uid = TEST_RES(mount_unique_id(LM_PARENT), _ret != 0);
	uint64_t child_uid = TEST_RES(mount_unique_id(LM_CHILD), _ret != 0);

	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = parent_uid,
	};
	uint64_t ids[16];
	TEST_RES(syscall(SYS_listmount, &req, ids, 16, 0), _ret == 1);
	TEST_RES(ids[0], _ret == child_uid);
}
END_TEST()

FN_TEST(listmount_cursor_paginates)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t first[16];
	long n_total =
		TEST_RES(syscall(SYS_listmount, &req, first, 16, 0), _ret >= 2);

	for (long i = 1; i < n_total; i++) {
		TEST_RES(first[i], _ret > first[i - 1]);
	}

	req.param = first[0];
	uint64_t rest[16];
	TEST_RES(syscall(SYS_listmount, &req, rest, 16, 0),
		 _ret == n_total - 1);
}
END_TEST()

FN_TEST(listmount_reverse_inverts_order)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t asc[16];
	long n_asc =
		TEST_RES(syscall(SYS_listmount, &req, asc, 16, 0), _ret >= 2);
	uint64_t desc[16];
	long n_desc = TEST_RES(syscall(SYS_listmount, &req, desc, 16,
				       LISTMOUNT_REVERSE),
			       _ret == n_asc);
	TEST_RES(asc[0], _ret == desc[n_desc - 1]);
	TEST_RES(asc[n_asc - 1], _ret == desc[0]);
}
END_TEST()

FN_TEST(listmount_rejects_unmounted_parent)
{
	uint64_t child_uid = TEST_RES(mount_unique_id(LM_CHILD), _ret != 0);
	TEST_SUCC(chdir(LM_CHILD));
	TEST_SUCC(umount(LM_CHILD));

	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = child_uid,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), ENOENT);

	TEST_SUCC(chdir("/"));
	TEST_SUCC(mount("tmpfs", LM_CHILD, "tmpfs", 0, NULL));
}
END_TEST()

FN_TEST(listmount_rejects_unknown_flags)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0xFFFFFFFEU), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_excessive_nr_mnt_ids)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1000001, 0), EOVERFLOW);
}
END_TEST()

FN_TEST(listmount_rejects_undersized_req)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0 - 1,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_oversized_req)
{
	// Pretending to be a future VER1-sized request must be rejected, not
	// silently truncated to VER0 semantics.
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0 + 8,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_nonzero_spare)
{
	// The second u32 is named `spare` in older headers and `mnt_ns_fd` in
	// newer ones.
	struct mnt_id_req req = {
		MNT_ID_REQ_SIZE_VER0,
		1,
		LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_invalid_mnt_ids_buffer)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	// A pointer past the user-space boundary fails the early buffer
	// validation, returning EFAULT before any namespace work.
	uint64_t *bogus = (uint64_t *)-1;
	TEST_ERRNO(syscall(SYS_listmount, &req, bogus, 16, 0), EFAULT);
}
END_TEST()

FN_TEST(listmount_rejects_out_of_range_mnt_id)
{
	// IDs below `MNT_UNIQUE_ID_MIN` are outside the unique mount ID range.
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = 0,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);

	req.mnt_id = MNT_UNIQUE_ID_MIN - 1;
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_out_of_range_param)
{
	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
		.param = MNT_UNIQUE_ID_MIN - 1,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(LM_CHILD));
	CHECK(rmdir(LM_CHILD));
	CHECK(umount(LM_PARENT));
	CHECK(rmdir(LM_PARENT));
}
END_SETUP()
