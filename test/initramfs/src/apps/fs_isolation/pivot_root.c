// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>

#include "../test.h"

// --- Test Configuration ---
#define CHROOT_DIR "/new_root"
#define PIVOT_TARGET_DIR "/second_root"
#define PUT_OLD_DIR_NAME "old_root"

// Marker directories and files to verify the pivot operation.
// These will be backed by tmpfs to ensure they are on distinct filesystems.
#define OLD_ROOT_MARKER_MNT "/old_root_marker_mnt"
#define OLD_ROOT_MARKER_FILE OLD_ROOT_MARKER_MNT "/old.txt"

#define NEW_ROOT_MARKER_MNT "/new_root_marker_mnt"
#define NEW_ROOT_MARKER_FILE NEW_ROOT_MARKER_MNT "/new.txt"

// Helper to create a directory if it doesn't exist.
static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), errno == 0 || errno == EEXIST);
}

// Helper to create a marker file on a tmpfs mount.
static void create_marker(const char *mount_point, const char *file_path)
{
	ensure_dir(mount_point);
	CHECK(mount("tmpfs", mount_point, "tmpfs", 0, ""));
	int fd = CHECK(open(file_path, O_CREAT | O_WRONLY, 0644));
	CHECK(close(fd));
}

FN_TEST(pivot_root_test)
{
	// --- Phase 1: Setup a chroot environment with a nested bind mount ---

	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir(CHROOT_DIR);
	TEST_SUCC(mount("/", CHROOT_DIR, NULL, MS_BIND | MS_REC, NULL));

	char pivot_target_full_path[256];
	snprintf(pivot_target_full_path, sizeof(pivot_target_full_path), "%s%s",
		 CHROOT_DIR, PIVOT_TARGET_DIR);
	ensure_dir(pivot_target_full_path);

	// Negative test: pivot_root should fail if the root mount is the root mount of rootfs.
	TEST_ERRNO(syscall(SYS_pivot_root, pivot_target_full_path,
			   pivot_target_full_path),
		   EINVAL);

	TEST_SUCC(chroot(CHROOT_DIR));
	TEST_SUCC(chdir("/")); // Change to the `new_root` after chroot.

	// --- Phase 2: Prepare for and execute pivot_root ---

	TEST_SUCC(mount("/", PIVOT_TARGET_DIR, NULL, MS_BIND | MS_REC, NULL));

	// Create the directory where the old root will be placed.
	char put_old_full_path[256];
	snprintf(put_old_full_path, sizeof(put_old_full_path), "%s/%s",
		 PIVOT_TARGET_DIR, PUT_OLD_DIR_NAME);
	TEST_SUCC(mkdir(put_old_full_path, 0755));
	// Mount a tmpfs on `put_old_full_path`. This makes it a different mount
	// from `new_root`.
	TEST_SUCC(mount("tmpfs", put_old_full_path, "tmpfs", 0, NULL));

	// Create marker files on separate tmpfs mounts to verify the pivot.
	create_marker(OLD_ROOT_MARKER_MNT, OLD_ROOT_MARKER_FILE);

	char new_root_marker_mnt_full_path[256];
	snprintf(new_root_marker_mnt_full_path,
		 sizeof(new_root_marker_mnt_full_path), "%s%s",
		 PIVOT_TARGET_DIR, NEW_ROOT_MARKER_MNT);
	char new_root_marker_file_full_path[256];
	snprintf(new_root_marker_file_full_path,
		 sizeof(new_root_marker_file_full_path), "%s%s",
		 PIVOT_TARGET_DIR, NEW_ROOT_MARKER_FILE);
	create_marker(new_root_marker_mnt_full_path,
		      new_root_marker_file_full_path);

	// --- Phase 3: Negative tests for pivot_root ---

	// pivot_root fails with ENOTDIR if the `new_root` or `put_old` is not a directory.
	TEST_ERRNO(syscall(SYS_pivot_root, new_root_marker_file_full_path,
			   put_old_full_path),
		   ENOTDIR);
	// pivot_root fails with EINVAL if the `put_old` is not underneath the `new_root`.
	TEST_ERRNO(syscall(SYS_pivot_root, "./proc", put_old_full_path),
		   EINVAL);
	// pivot_root fails with EINVAL if the `new_root` is not a mount root.
	TEST_ERRNO(syscall(SYS_pivot_root, "./sys/fs", "./sys/fs"), EINVAL);
	// pivot_root fails with EBUSY if the `new_root` is the current root.
	TEST_ERRNO(syscall(SYS_pivot_root, ".", "./bin"), EBUSY);

	// --- Phase 4: Do pivot_root and verification ---
	// Perform the pivot_root operation.
	TEST_SUCC(syscall(SYS_pivot_root, PIVOT_TARGET_DIR, put_old_full_path));

	// After pivot, the cwd is changed to the `new_root`.
	char cwd[1024];
	TEST_RES(syscall(SYS_getcwd, cwd, sizeof(cwd)), strcmp(cwd, "/") == 0);

	// Verify that the `new_root` is active by checking for its marker file.
	TEST_SUCC(access(NEW_ROOT_MARKER_FILE, F_OK));

	// Verify that the old root has been moved.
	char old_marker_in_new_path[256];
	snprintf(old_marker_in_new_path, sizeof(old_marker_in_new_path),
		 "/%s%s", PUT_OLD_DIR_NAME, OLD_ROOT_MARKER_FILE);
	TEST_SUCC(access(old_marker_in_new_path, F_OK));

	// Verify that the old root marker is no longer at the root.
	TEST_ERRNO(access(OLD_ROOT_MARKER_FILE, F_OK), ENOENT);

	// --- Phase 4: Cleanup ---

	char old_root_path[256];
	snprintf(old_root_path, sizeof(old_root_path), "/%s", PUT_OLD_DIR_NAME);
	TEST_SUCC(umount2(old_root_path, MNT_DETACH));
	TEST_SUCC(umount(old_root_path));
	TEST_SUCC(rmdir(old_root_path));
}
END_TEST()