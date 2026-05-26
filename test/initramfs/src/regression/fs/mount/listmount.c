// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <stdint.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#ifndef SYS_listmount
#define SYS_listmount 458
#endif

#ifndef STATX_MNT_ID_UNIQUE
#define STATX_MNT_ID_UNIQUE 0x4000U
#endif

struct linux_mnt_id_req {
	uint32_t size;
	uint32_t spare;
	uint64_t mnt_id;
	uint64_t param;
};

#ifndef MNT_ID_REQ_SIZE_VER0
#define MNT_ID_REQ_SIZE_VER0 24
#endif

#ifndef LSMT_ROOT
#define LSMT_ROOT 0xffffffffffffffffULL
#endif

#ifndef LISTMOUNT_REVERSE
#define LISTMOUNT_REVERSE (1U << 0)
#endif

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

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
	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir("/tmp/lm_root");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_root", "tmpfs", 0, NULL));

	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[16];
	TEST_RES(syscall(SYS_listmount, &req, ids, 16, 0), _ret >= 1);

	TEST_SUCC(umount("/tmp/lm_root"));
}
END_TEST()

FN_TEST(listmount_lists_descendants_of_specific_parent)
{
	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir("/tmp/lm_parent");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_parent", "tmpfs", 0, NULL));
	ensure_dir("/tmp/lm_parent/child");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_parent/child", "tmpfs", 0, NULL));

	uint64_t parent_uid = mount_unique_id("/tmp/lm_parent");
	uint64_t child_uid = mount_unique_id("/tmp/lm_parent/child");
	SKIP_TEST_IF(parent_uid == 0 || child_uid == 0);

	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = parent_uid,
	};
	uint64_t ids[16];
	long n = TEST_RES(syscall(SYS_listmount, &req, ids, 16, 0), _ret == 1);
	if (n == 1) {
		TEST_RES(ids[0], _ret == child_uid);
	}

	TEST_SUCC(umount("/tmp/lm_parent/child"));
	TEST_SUCC(umount("/tmp/lm_parent"));
}
END_TEST()

FN_TEST(listmount_cursor_paginates)
{
	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir("/tmp/lm_p1");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_p1", "tmpfs", 0, NULL));
	ensure_dir("/tmp/lm_p2");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_p2", "tmpfs", 0, NULL));

	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t first[16];
	long n_total =
		TEST_RES(syscall(SYS_listmount, &req, first, 16, 0), _ret >= 2);
	if (n_total < 2) {
		goto cleanup;
	}

	for (long i = 1; i < n_total; i++) {
		TEST_RES(first[i], _ret > first[i - 1]);
	}

	req.param = first[0];
	uint64_t rest[16];
	TEST_RES(syscall(SYS_listmount, &req, rest, 16, 0),
		 _ret == n_total - 1);

cleanup:
	TEST_SUCC(umount("/tmp/lm_p1"));
	TEST_SUCC(umount("/tmp/lm_p2"));
}
END_TEST()

FN_TEST(listmount_reverse_inverts_order)
{
	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir("/tmp/lm_r1");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_r1", "tmpfs", 0, NULL));
	ensure_dir("/tmp/lm_r2");
	TEST_SUCC(mount("tmpfs", "/tmp/lm_r2", "tmpfs", 0, NULL));

	struct linux_mnt_id_req req = {
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
	if (n_asc >= 2 && n_asc == n_desc) {
		TEST_RES(asc[0], _ret == desc[n_desc - 1]);
		TEST_RES(asc[n_asc - 1], _ret == desc[0]);
	}

	TEST_SUCC(umount("/tmp/lm_r1"));
	TEST_SUCC(umount("/tmp/lm_r2"));
}
END_TEST()

FN_TEST(listmount_rejects_unknown_flags)
{
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0xFFFFFFFEU), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_excessive_nr_mnt_ids)
{
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1000001, 0), EOVERFLOW);
}
END_TEST()

FN_TEST(listmount_rejects_undersized_req)
{
	struct linux_mnt_id_req req = {
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
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0 + 8,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_nonzero_spare)
{
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.spare = 1,
		.mnt_id = LSMT_ROOT,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), EINVAL);
}
END_TEST()

FN_TEST(listmount_rejects_invalid_mnt_ids_buffer)
{
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = LSMT_ROOT,
	};
	// A pointer past the user-space boundary fails the early buffer
	// validation, returning EFAULT before any namespace work.
	uint64_t *bogus = (uint64_t *)-1;
	TEST_ERRNO(syscall(SYS_listmount, &req, bogus, 16, 0), EFAULT);
}
END_TEST()

FN_TEST(listmount_rejects_unknown_mnt_id)
{
	// A `unique_id` of 0 can never be issued (the counter starts above
	// `MNT_UNIQUE_ID_OFFSET`), so the lookup must miss.
	struct linux_mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = 0,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), ENOENT);
}
END_TEST()
