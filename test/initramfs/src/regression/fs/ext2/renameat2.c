// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/fs.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/renameat2_test"
#define OUT_FILE BASE_DIR "/out.txt"

// do_renameat2 wrapper via raw syscall, portable across libc versions.
static int do_renameat2(int olddirfd, const char *oldpath, int newdirfd,
			const char *newpath, unsigned int flags)
{
	return (int)syscall(__NR_renameat2, olddirfd, oldpath, newdirfd,
			    newpath, flags);
}

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void remove_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
}

static void unlink_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
}

static void write_file(const char *path, const char *content)
{
	int fd = CHECK_WITH(open(path, O_CREAT | O_WRONLY | O_TRUNC, 0644),
			    _ret >= 0);
	ssize_t len = (ssize_t)strlen(content);
	ssize_t n = CHECK_WITH(write(fd, content, (size_t)len), _ret >= 0);
	if (n != len) {
		fprintf(stderr, "write_file short write\n");
		exit(EXIT_FAILURE);
	}
	CHECK_WITH(close(fd), _ret >= 0);
}

static void read_file_expect(const char *path, const char *expected)
{
	char buf[256];
	int fd = CHECK_WITH(open(path, O_RDONLY), _ret >= 0);
	ssize_t n = CHECK_WITH(read(fd, buf, sizeof(buf) - 1), _ret >= 0);
	CHECK_WITH(close(fd), _ret >= 0);
	buf[n] = '\0';
	if (strcmp(buf, expected) != 0) {
		fprintf(stderr, "read_file_expect: got '%s', expected '%s'\n",
			buf, expected);
		exit(EXIT_FAILURE);
	}
}

/* ------------------------------------------------------------------ */
/* Setup                                                               */
/* ------------------------------------------------------------------ */

FN_SETUP(cleanup)
{
	unlink_if_exists(OUT_FILE);
	remove_if_exists(BASE_DIR);
}
END_SETUP()

/* ------------------------------------------------------------------ */
/* 1. Same-directory file↔file exchange                                */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_same_dir_files)
{
	ensure_dir(BASE_DIR);
	const char *f1 = BASE_DIR "/f1";
	const char *f2 = BASE_DIR "/f2";

	write_file(f1, "aaa");
	write_file(f2, "bbb");

	TEST_SUCC(do_renameat2(AT_FDCWD, f1, AT_FDCWD, f2, RENAME_EXCHANGE));

	read_file_expect(f1, "bbb");
	read_file_expect(f2, "aaa");

	unlink_if_exists(f1);
	unlink_if_exists(f2);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 2. Cross-directory file↔file exchange — content + parent nlink      */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_cross_dir_files)
{
	ensure_dir(BASE_DIR);
	const char *da = BASE_DIR "/da";
	const char *db = BASE_DIR "/db";
	const char *fa = BASE_DIR "/da/fa";
	const char *fb = BASE_DIR "/db/fb";
	ensure_dir(da);
	ensure_dir(db);

	write_file(fa, "hello");
	write_file(fb, "world");

	struct stat st_a_before, st_b_before;
	TEST_SUCC(stat(da, &st_a_before));
	TEST_SUCC(stat(db, &st_b_before));

	TEST_SUCC(do_renameat2(AT_FDCWD, fa, AT_FDCWD, fb, RENAME_EXCHANGE));

	read_file_expect(fa, "world");
	read_file_expect(fb, "hello");

	// Parent directory link counts must not change (both are non-dirs).
	struct stat st_a_after, st_b_after;
	TEST_SUCC(stat(da, &st_a_after));
	TEST_SUCC(stat(db, &st_b_after));
	TEST_RES(stat(da, &st_a_after),
		 st_a_after.st_nlink == st_a_before.st_nlink);
	TEST_RES(stat(db, &st_b_after),
		 st_b_after.st_nlink == st_b_before.st_nlink);

	unlink_if_exists(fa);
	unlink_if_exists(fb);
	remove_if_exists(da);
	remove_if_exists(db);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 3. Cross-directory dir↔dir exchange — .. + parent nlink unchanged    */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_cross_dir_dirs)
{
	ensure_dir(BASE_DIR);
	const char *da = BASE_DIR "/da";
	const char *db = BASE_DIR "/db";
	const char *da_child = BASE_DIR "/da/src";
	const char *db_child = BASE_DIR "/db/dst";
	ensure_dir(da);
	ensure_dir(db);
	ensure_dir(da_child);
	ensure_dir(db_child);

	struct stat st_a_before, st_b_before;
	TEST_SUCC(stat(da, &st_a_before));
	TEST_SUCC(stat(db, &st_b_before));

	TEST_SUCC(do_renameat2(AT_FDCWD, da_child, AT_FDCWD, db_child,
			       RENAME_EXCHANGE));

	// Exchange swaps the inodes that the entries point to, not the entry
	// names. Both paths still exist, just under opposite parents.
	TEST_SUCC(access(BASE_DIR "/da/src", F_OK));
	TEST_SUCC(access(BASE_DIR "/db/dst", F_OK));

	// Net parent link counts unchanged: each loses one child dir's ..
	// and gains the other's.
	struct stat st_a_after, st_b_after;
	TEST_SUCC(stat(da, &st_a_after));
	TEST_SUCC(stat(db, &st_b_after));
	TEST_RES(stat(da, &st_a_after),
		 st_a_after.st_nlink == st_a_before.st_nlink);
	TEST_RES(stat(db, &st_b_after),
		 st_b_after.st_nlink == st_b_before.st_nlink);

	remove_if_exists(BASE_DIR "/da/src");
	remove_if_exists(BASE_DIR "/db/dst");
	remove_if_exists(da);
	remove_if_exists(db);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 4. Cross-directory dir→file exchange (single-dir, aligns Linux)      */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_cross_dir_dir_with_file)
{
	ensure_dir(BASE_DIR);
	const char *da = BASE_DIR "/da";
	const char *db = BASE_DIR "/db";
	const char *da_child = BASE_DIR "/da/src"; // directory
	const char *db_file = BASE_DIR "/db/dst"; // regular file
	ensure_dir(da);
	ensure_dir(db);
	ensure_dir(da_child);
	write_file(db_file, "content");

	struct stat st_a_before, st_b_before;
	TEST_SUCC(stat(da, &st_a_before));
	TEST_SUCC(stat(db, &st_b_before));

	// dir ↔ file must succeed — same as Linux and ramfs.
	TEST_SUCC(do_renameat2(AT_FDCWD, da_child, AT_FDCWD, db_file,
			       RENAME_EXCHANGE));

	// After exchange: da/src is now a regular file, db/dst is a dir.
	TEST_SUCC(access(BASE_DIR "/da/src", F_OK));
	TEST_SUCC(access(BASE_DIR "/db/dst", F_OK));

	// Parent nlink: da loses one child dir (src left), db gains one
	// child dir (dst arrived).
	struct stat st_a_after, st_b_after;
	TEST_SUCC(stat(da, &st_a_after));
	TEST_SUCC(stat(db, &st_b_after));
	TEST_RES(stat(da, &st_a_after),
		 st_a_after.st_nlink == st_a_before.st_nlink - 1);
	TEST_RES(stat(db, &st_b_after),
		 st_b_after.st_nlink == st_b_before.st_nlink + 1);

	unlink_if_exists(BASE_DIR "/da/src");
	remove_if_exists(BASE_DIR "/db/dst");
	remove_if_exists(da);
	remove_if_exists(db);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 5. Cross-directory file→dir exchange (single-dir, symmetric of #4)   */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_cross_dir_file_with_dir)
{
	ensure_dir(BASE_DIR);
	const char *da = BASE_DIR "/da";
	const char *db = BASE_DIR "/db";
	const char *da_file = BASE_DIR "/da/src"; // regular file
	const char *db_child = BASE_DIR "/db/dst"; // directory
	ensure_dir(da);
	ensure_dir(db);
	write_file(da_file, "data");
	ensure_dir(db_child);

	struct stat st_a_before, st_b_before;
	TEST_SUCC(stat(da, &st_a_before));
	TEST_SUCC(stat(db, &st_b_before));

	// file ↔ dir — symmetric of case 4.
	TEST_SUCC(do_renameat2(AT_FDCWD, da_file, AT_FDCWD, db_child,
			       RENAME_EXCHANGE));

	// Parent nlink: da gains one dir (dst arrived), db loses one dir
	// (dst left).
	struct stat st_a_after, st_b_after;
	TEST_SUCC(stat(da, &st_a_after));
	TEST_SUCC(stat(db, &st_b_after));
	TEST_RES(stat(da, &st_a_after),
		 st_a_after.st_nlink == st_a_before.st_nlink + 1);
	TEST_RES(stat(db, &st_b_after),
		 st_b_after.st_nlink == st_b_before.st_nlink - 1);

	unlink_if_exists(BASE_DIR "/db/dst");
	remove_if_exists(BASE_DIR "/da/src");
	remove_if_exists(da);
	remove_if_exists(db);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 6. NOREPLACE — destination exists → EEXIST                          */
/* ------------------------------------------------------------------ */
FN_TEST(noreplace_target_exists)
{
	ensure_dir(BASE_DIR);
	const char *f1 = BASE_DIR "/f1";
	const char *f2 = BASE_DIR "/f2";
	write_file(f1, "x");
	write_file(f2, "y");

	TEST_ERRNO(do_renameat2(AT_FDCWD, f1, AT_FDCWD, f2, RENAME_NOREPLACE),
		   EEXIST);

	// Neither file should have been modified.
	read_file_expect(f1, "x");
	read_file_expect(f2, "y");

	unlink_if_exists(f1);
	unlink_if_exists(f2);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 7. EXCHANGE — destination does not exist → ENOENT                    */
/* ------------------------------------------------------------------ */
FN_TEST(exchange_target_missing)
{
	ensure_dir(BASE_DIR);
	const char *f1 = BASE_DIR "/f1";
	const char *missing = BASE_DIR "/no_such";
	write_file(f1, "x");

	TEST_ERRNO(do_renameat2(AT_FDCWD, f1, AT_FDCWD, missing,
				RENAME_EXCHANGE),
		   ENOENT);

	read_file_expect(f1, "x");
	unlink_if_exists(f1);
	remove_if_exists(BASE_DIR);
}
END_TEST()

/* ------------------------------------------------------------------ */
/* 8. NOREPLACE | EXCHANGE → EINVAL                                    */
/* ------------------------------------------------------------------ */
FN_TEST(noreplace_and_exchange_einval)
{
	ensure_dir(BASE_DIR);
	const char *f1 = BASE_DIR "/f1";
	const char *f2 = BASE_DIR "/f2";
	write_file(f1, "a");
	write_file(f2, "b");

	TEST_ERRNO(do_renameat2(AT_FDCWD, f1, AT_FDCWD, f2,
				RENAME_NOREPLACE | RENAME_EXCHANGE),
		   EINVAL);

	unlink_if_exists(f1);
	unlink_if_exists(f2);
	remove_if_exists(BASE_DIR);
}
END_TEST()
