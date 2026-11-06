// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_lower_metacopy"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

/*
 * The metadata-only record chain that lives entirely in lower layers: a record
 * at one layer and the data object at a deeper one, with the upper layer taking
 * no part in the fixture. Every case therefore observes how a mount resolves an
 * object it never copied up.
 *
 * Each record is a LOWER_DATA_LEN-byte all-zero regular file carrying the v0
 * record, and each data object holds the literal LOWER_DATA. The expected bytes
 * can never come from the record: a mount that serves the record reads zeros,
 * so positive cases compare against a literal and a zero serve cannot match.
 * Negative cases assert an errno the pre-fix kernel does not return, so none of
 * them can pass vacuously.
 *
 * Case 8 reads the identity case 7 captured and the identity case 5b observed
 * before its first promotion, so those two values travel between test
 * functions. The constructors run in definition order, which is the order the
 * cases are written in.
 */

/* Three lower layers (t1, t2, bottom) that follow records. */
#define ON_ROOT BASE_DIR "/on"

/* Two lower layers (top, bottom) that refuse records. */
#define OFF_ROOT BASE_DIR "/off"

#define ON_OPTIONS ",metacopy=on,redirect_dir=on"
#define OFF_OPTIONS ",metacopy=off,redirect_dir=nofollow"

#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN (sizeof(LOWER_DATA) - 1)

/* A name only the bottom layer of case 3's parent holds. */
#define BOTTOM_ONLY "bottom-only"
#define BOTTOM_ONLY_LEN (sizeof(BOTTOM_ONLY) - 1)

/* The body LOWER_DATA becomes after one byte is written at offset 0. */
#define WRITTEN_DATA "Xower-data"
#define WRITTEN_DATA_LEN (sizeof(WRITTEN_DATA) - 1)

static ino_t lm05b_ino_before;
static ino_t lm07r_ino_before;

/*
 * Prints the stat fields a case reports, so a failed assertion shows the value
 * it observed and not only the boolean it wanted.
 */
static void report_stat(const char *tag, const struct stat *st)
{
	fprintf(stderr,
		"    observed %s: mode=%#o size=%lld nlink=%llu ino=%llu dev=%llu\n",
		tag, (unsigned)(st->st_mode & S_IFMT), (long long)st->st_size,
		(unsigned long long)st->st_nlink,
		(unsigned long long)st->st_ino, (unsigned long long)st->st_dev);
}

/*
 * Prints the `len` bytes at `buf` in hex under `tag`. A record's body is zeros,
 * which the string form of `report_body` renders as empty, so this is the form
 * that makes "the upper never holds zeros" auditable from the log.
 */
static void report_hex(const char *tag, const char *buf, ssize_t len)
{
	size_t i;

	fprintf(stderr, "    observed %s: len=%lld hex=", tag, (long long)len);
	for (i = 0; i < (size_t)(len < 0 ? 0 : len); i++)
		fprintf(stderr, "%02x", (unsigned char)buf[i]);
	fprintf(stderr, "\n");
}

/*
 * Returns 1 when `path` cannot be accessed at all, 0 otherwise. `errno` is
 * cleared before returning, so the result can be checked with `TEST_RES` even
 * though the probe itself fails.
 */
static int path_unreachable(const char *path)
{
	int unreachable = access(path, F_OK) != 0;

	errno = 0;
	return unreachable;
}

/* Case 1: a record at t1, the data at bottom. */
static void build_two_layer_chain_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm01", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm01");
	write_file(ON_ROOT "/bottom/lm01", LOWER_DATA);
}

/* Case 2: consecutive records at t1 and t2, the data at bottom. */
static void build_three_layer_chain_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm02", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm02");
	write_zero_file(ON_ROOT "/t2/lm02", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t2/lm02");
	write_file(ON_ROOT "/bottom/lm02", LOWER_DATA);
}

/*
 * Case 3: the chain sits under a parent directory that t1 holds empty, t2 holds
 * with the record inside, and bottom holds with the data and one name of its
 * own. Every parent is created explicitly so all three layers join the parent's
 * own stack.
 */
static void build_merged_parent_chain_fixture(void)
{
	create_dir(ON_ROOT "/t1/lm03");
	create_dir(ON_ROOT "/t2/lm03");
	write_zero_file(ON_ROOT "/t2/lm03/f", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t2/lm03/f");
	create_dir(ON_ROOT "/bottom/lm03");
	write_file(ON_ROOT "/bottom/lm03/f", LOWER_DATA);
	write_file(ON_ROOT "/bottom/lm03/g", BOTTOM_ONLY);
}

/*
 * Case 4: the data position holds a directory, and a second name's data
 * position holds a symlink. Neither is a regular file, so neither can be the
 * object's data.
 */
static void build_non_regular_data_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm04", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm04");
	create_dir(ON_ROOT "/bottom/lm04");

	write_zero_file(ON_ROOT "/t1/lm04s", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm04s");
	write_file(ON_ROOT "/bottom/lm04data", LOWER_DATA);
	CHECK(symlink("lm04data", ON_ROOT "/bottom/lm04s"));
}

/* Case 5a: a writable open completes the data from the record's data object. */
static void build_writable_open_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm05", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm05");
	write_file(ON_ROOT "/bottom/lm05", LOWER_DATA);
}

/* Case 5b: the same shape, promoted metadata first and data second. */
static void build_two_step_promotion_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm05b", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm05b");
	write_file(ON_ROOT "/bottom/lm05b", LOWER_DATA);
}

/* Case 6: the same record shape on the mount that does not follow records. */
static void build_policy_gate_fixture(void)
{
	write_zero_file(OFF_ROOT "/top/off01", LOWER_DATA_LEN);
	set_metacopy(OFF_ROOT "/top/off01");
	write_file(OFF_ROOT "/bottom/off01", LOWER_DATA);
}

/* Case 7: two chains in the merged root, one unlinked and one renamed. */
static void build_unlink_and_rename_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm07u", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm07u");
	write_file(ON_ROOT "/bottom/lm07u", LOWER_DATA);

	write_zero_file(ON_ROOT "/t1/lm07r", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm07r");
	write_file(ON_ROOT "/bottom/lm07r", LOWER_DATA);
}

/* Case 8: the case 5b promotion sequence on a name of its own. */
static void build_identity_fixture(void)
{
	write_zero_file(ON_ROOT "/t1/lm08", LOWER_DATA_LEN);
	set_metacopy(ON_ROOT "/t1/lm08");
	write_file(ON_ROOT "/bottom/lm08", LOWER_DATA);
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_three_lower_tree_at(ON_ROOT);
	create_multi_lower_tree_at(OFF_ROOT);

	build_two_layer_chain_fixture();
	build_three_layer_chain_fixture();
	build_merged_parent_chain_fixture();
	build_non_regular_data_fixture();
	build_writable_open_fixture();
	build_two_step_promotion_fixture();
	build_policy_gate_fixture();
	build_unlink_and_rename_fixture();
	build_identity_fixture();

	mount_overlay_three_lower_at(ON_ROOT, ON_OPTIONS);
	mount_overlay_multi_lower_at(OFF_ROOT, OFF_OPTIONS);
}

END_SETUP()

FN_TEST(lm01_two_layer_chain)
{
	char buf[64];
	struct stat st;
	ssize_t len;

	/* The data may only come from bottom; the record's body is 10 zeros. */
	len = read_body(ON_ROOT "/merged/lm01", buf, sizeof(buf));
	report_body("merged lm01", buf, len);
	report_hex("merged lm01", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	/* The record carries the data's size, so this row only pins the shape. */
	TEST_RES(lstat(ON_ROOT "/merged/lm01", &st), _ret == 0);
	report_stat("merged lm01", &st);
	TEST_RES((st.st_mode & S_IFMT) == S_IFREG, _ret == 1);
	TEST_RES(st.st_size == LOWER_DATA_LEN, _ret == 1);
}

END_TEST()

FN_TEST(lm02_three_layer_chain)
{
	char buf[64];
	struct stat st;
	ssize_t len;

	/* The data scan skips the intermediate record and accepts bottom. */
	len = read_body(ON_ROOT "/merged/lm02", buf, sizeof(buf));
	report_body("merged lm02", buf, len);
	report_hex("merged lm02", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_RES(lstat(ON_ROOT "/merged/lm02", &st), _ret == 0);
	report_stat("merged lm02", &st);
	TEST_RES((st.st_mode & S_IFMT) == S_IFREG, _ret == 1);
	TEST_RES(st.st_size == LOWER_DATA_LEN, _ret == 1);

	/* Resolving the chain must not rewrite either layer it read. */
	TEST_RES(metacopy_present(ON_ROOT "/t1/lm02"), _ret == 1);
	TEST_RES(metacopy_present(ON_ROOT "/t2/lm02"), _ret == 1);
}

END_TEST()

FN_TEST(lm03_chain_under_merged_parent)
{
	char buf[64];
	struct stat st, again;
	ssize_t len;

	/* The chain's two-element stack is invisible to the parent's merge. */
	TEST_RES(find_entry(ON_ROOT "/merged/lm03", "f"), _ret == 1);
	TEST_RES(find_entry(ON_ROOT "/merged/lm03", "g"), _ret == 1);

	len = read_body(ON_ROOT "/merged/lm03/f", buf, sizeof(buf));
	report_body("merged lm03/f", buf, len);
	report_hex("merged lm03/f", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	/* A plain bottom file keeps reading from bottom. */
	len = read_body(ON_ROOT "/merged/lm03/g", buf, sizeof(buf));
	report_body("merged lm03/g", buf, len);
	TEST_RES(len, _ret == BOTTOM_ONLY_LEN);
	TEST_RES(strcmp(buf, BOTTOM_ONLY), _ret == 0);

	/* The parent's own identity is the same on every lookup. */
	TEST_RES(stat(ON_ROOT "/merged/lm03", &st), _ret == 0);
	TEST_RES(stat(ON_ROOT "/merged/lm03", &again), _ret == 0);
	report_stat("merged lm03 first", &st);
	report_stat("merged lm03 second", &again);
	TEST_RES(st.st_ino == again.st_ino, _ret == 1);
}

END_TEST()

FN_TEST(lm04_non_regular_data_position)
{
	struct stat st;

	/*
	 * A directory in the data position names no data object, so the mount
	 * reports the run-time error instead of publishing the record.
	 */
	TEST_ERRNO(lstat(ON_ROOT "/merged/lm04", &st), EIO);
	TEST_ERRNO(open(ON_ROOT "/merged/lm04", O_RDONLY), EIO);

	/* A symlink in the data position is the same refusal. */
	TEST_ERRNO(open(ON_ROOT "/merged/lm04s", O_RDONLY), EIO);

	/* None of the failed lookups published a name. */
	TEST_RES(path_unreachable(ON_ROOT "/merged/lm04"), _ret == 1);
}

END_TEST()

FN_TEST(lm05_writable_open_completes_data)
{
	char buf[64];
	struct stat st;
	ssize_t len;
	int fd;

	/* The writable open is what completes the deferred data copy. */
	fd = TEST_SUCC(open(ON_ROOT "/merged/lm05", O_RDWR));
	TEST_SUCC(close(fd));

	/* The upper must hold the data object's bytes, never the record's zeros. */
	len = read_body(ON_ROOT "/upper/lm05", buf, sizeof(buf));
	report_body("upper lm05", buf, len);
	report_hex("upper lm05", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	/* A full copy-up is a copy of the data, so the record falls away. */
	TEST_RES(metacopy_absent(ON_ROOT "/upper/lm05"), _ret == 1);

	len = read_body(ON_ROOT "/merged/lm05", buf, sizeof(buf));
	report_body("merged lm05", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_RES(lstat(ON_ROOT "/upper/lm05", &st), _ret == 0);
	report_stat("upper lm05", &st);
	TEST_RES(st.st_size == LOWER_DATA_LEN, _ret == 1);
}

END_TEST()

FN_TEST(lm05b_two_step_promotion)
{
	char buf[64];
	struct stat st;
	ssize_t len;
	int fd;

	/* The record's own number, read before any promotion moves the object. */
	TEST_RES(stat(ON_ROOT "/merged/lm05b", &st), _ret == 0);
	lm05b_ino_before = st.st_ino;
	report_stat("merged lm05b before chmod", &st);

	/*
	 * Step one: chmod promotes the metadata only, so the upper becomes a
	 * record and must not become the object's data source as well.
	 */
	TEST_SUCC(chmod(ON_ROOT "/merged/lm05b", 0600));
	TEST_RES(metacopy_present(ON_ROOT "/upper/lm05b"), _ret == 1);

	/* The upper now carries the record: its body is the fixture's zeros. */
	len = read_body(ON_ROOT "/upper/lm05b", buf, sizeof(buf));
	report_body("upper lm05b before completion", buf, len);
	report_hex("upper lm05b before completion", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);

	len = read_body(ON_ROOT "/merged/lm05b", buf, sizeof(buf));
	report_body("merged lm05b after metadata promotion", buf, len);
	report_hex("merged lm05b after metadata promotion", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_RES(lstat(ON_ROOT "/merged/lm05b", &st), _ret == 0);
	report_stat("merged lm05b after metadata promotion", &st);
	TEST_RES(st.st_size == LOWER_DATA_LEN, _ret == 1);
	/* Metadata-only promotion does not move the published identity. */
	TEST_RES(st.st_ino == lm05b_ino_before, _ret == 1);

	/*
	 * Step two: the writable open completes the data. Its bytes must come
	 * from the data object, never from the record the upper now holds.
	 */
	fd = TEST_SUCC(open(ON_ROOT "/merged/lm05b", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(metacopy_absent(ON_ROOT "/upper/lm05b"), _ret == 1);

	len = read_body(ON_ROOT "/upper/lm05b", buf, sizeof(buf));
	report_body("upper lm05b after completion", buf, len);
	report_hex("upper lm05b after completion", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_RES(stat(ON_ROOT "/merged/lm05b", &st), _ret == 0);
	report_stat("merged lm05b after completion", &st);
	TEST_RES(st.st_ino == lm05b_ino_before, _ret == 1);

	/*
	 * The write path never publishes zeros: the first byte written through
	 * the name lands on the copied data, not on a zero-filled body.
	 */
	fd = TEST_SUCC(open(ON_ROOT "/merged/lm05b", O_WRONLY));
	TEST_RES(write(fd, "X", 1), _ret == 1);
	TEST_SUCC(close(fd));

	len = read_body(ON_ROOT "/merged/lm05b", buf, sizeof(buf));
	report_body("merged lm05b after one byte", buf, len);
	report_hex("merged lm05b after one byte", buf, len);
	TEST_RES(len, _ret == WRITTEN_DATA_LEN);
	TEST_RES(strcmp(buf, WRITTEN_DATA), _ret == 0);
}

END_TEST()

FN_TEST(off01_metacopy_off_refuses_lower_record)
{
	char buf[64];
	struct stat st;
	ssize_t len;

	/* With metacopy=off a lower record is refused, not served. */
	TEST_ERRNO(lstat(OFF_ROOT "/merged/off01", &st), EPERM);
	TEST_ERRNO(open(OFF_ROOT "/merged/off01", O_RDONLY), EPERM);
	TEST_ERRNO(open(OFF_ROOT "/merged/off01", O_RDWR), EPERM);

	/* The gate is the mount's, not the file's: the layer still reads. */
	len = read_body(OFF_ROOT "/top/off01", buf, sizeof(buf));
	report_body("layer top/off01", buf, len);
	report_hex("layer top/off01", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
}

END_TEST()

FN_TEST(lm07_unlink_and_rename_invariants)
{
	char buf[64];
	struct stat st, renamed;
	ssize_t len;

	/* The object exists and reads before the unlink. */
	len = read_body(ON_ROOT "/merged/lm07u", buf, sizeof(buf));
	report_body("merged lm07u", buf, len);
	report_hex("merged lm07u", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	/* The whiteout decision reads the parent's layers, not the chain. */
	TEST_SUCC(unlink(ON_ROOT "/merged/lm07u"));
	TEST_ERRNO(lstat(ON_ROOT "/merged/lm07u", &st), ENOENT);
	TEST_RES(read_body(ON_ROOT "/merged/lm07u", buf, sizeof(buf)),
		 _ret == -1);

	/* Both layers the unlink read stay untouched, and the upper hides it. */
	TEST_RES(lstat(ON_ROOT "/t1/lm07u", &st), _ret == 0);
	TEST_RES(lstat(ON_ROOT "/bottom/lm07u", &st), _ret == 0);
	TEST_RES(lstat(ON_ROOT "/upper/lm07u", &st), _ret == 0);

	/* The rename moves the object itself; the new name still reads data. */
	TEST_RES(stat(ON_ROOT "/merged/lm07r", &st), _ret == 0);
	lm07r_ino_before = st.st_ino;
	report_stat("merged lm07r before rename", &st);
	TEST_SUCC(rename(ON_ROOT "/merged/lm07r", ON_ROOT "/merged/lm07s"));
	TEST_ERRNO(lstat(ON_ROOT "/merged/lm07r", &st), ENOENT);
	TEST_RES(lstat(ON_ROOT "/upper/lm07s", &st), _ret == 0);

	len = read_body(ON_ROOT "/merged/lm07s", buf, sizeof(buf));
	report_body("merged lm07s", buf, len);
	report_hex("merged lm07s", buf, len);
	TEST_RES(len, _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);

	TEST_RES(stat(ON_ROOT "/merged/lm07s", &renamed), _ret == 0);
	report_stat("merged lm07s after rename", &renamed);
	TEST_RES(renamed.st_ino == lm07r_ino_before, _ret == 1);
}

END_TEST()

FN_TEST(lm08_identity_is_the_records)
{
	struct stat merged_st, record_st, data_st, promoted_st;
	ino_t before;
	int fd;

	/*
	 * The same-filesystem guard: every layer of the fixture lives on the
	 * initramfs filesystem, so the mount publishes the layers' own pairs and
	 * the record's inode is expressible from user space. When the guard does
	 * not hold, that inode cannot be computed here and the row is reported
	 * NOTRUN rather than passed.
	 */
	TEST_RES(stat(ON_ROOT "/merged/lm01", &merged_st), _ret == 0);
	TEST_RES(lstat(ON_ROOT "/t1/lm01", &record_st), _ret == 0);
	TEST_RES(lstat(ON_ROOT "/bottom/lm01", &data_st), _ret == 0);
	report_stat("merged lm01", &merged_st);
	report_stat("layer t1/lm01 (the record)", &record_st);
	report_stat("layer bottom/lm01 (the data)", &data_st);
	if (merged_st.st_dev == record_st.st_dev) {
		/* The published pair is the record's, never the data object's. */
		TEST_RES(merged_st.st_ino == record_st.st_ino, _ret == 1);
		TEST_RES(merged_st.st_ino != data_st.st_ino, _ret == 1);
	} else {
		fprintf(stderr,
			"    lm08: 8.1 NOTRUN [merged st_dev=%llu, record st_dev=%llu]\n",
			(unsigned long long)merged_st.st_dev,
			(unsigned long long)record_st.st_dev);
	}

	/* The identity must not move when the promotions replace the source. */
	TEST_RES(stat(ON_ROOT "/merged/lm08", &promoted_st), _ret == 0);
	before = promoted_st.st_ino;
	report_stat("merged lm08 before chmod", &promoted_st);

	TEST_SUCC(chmod(ON_ROOT "/merged/lm08", 0600));
	TEST_RES(stat(ON_ROOT "/merged/lm08", &promoted_st), _ret == 0);
	report_stat("merged lm08 after chmod", &promoted_st);
	TEST_RES(promoted_st.st_ino == before, _ret == 1);

	fd = TEST_SUCC(open(ON_ROOT "/merged/lm08", O_RDWR));
	TEST_SUCC(close(fd));
	TEST_RES(stat(ON_ROOT "/merged/lm08", &promoted_st), _ret == 0);
	report_stat("merged lm08 after O_RDWR", &promoted_st);
	TEST_RES(promoted_st.st_ino == before, _ret == 1);

	/* The same stability across the two promotions case 5b performed. */
	TEST_RES(stat(ON_ROOT "/merged/lm05b", &promoted_st), _ret == 0);
	report_stat("merged lm05b after its two promotions", &promoted_st);
	TEST_RES(promoted_st.st_ino == lm05b_ino_before, _ret == 1);

	/* The rename of case 7 moved the name, not the identity. */
	TEST_RES(stat(ON_ROOT "/merged/lm07s", &promoted_st), _ret == 0);
	report_stat("merged lm07s after its rename", &promoted_st);
	TEST_RES(promoted_st.st_ino == lm07r_ino_before, _ret == 1);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(OFF_ROOT);
	unmount_overlay_at(ON_ROOT);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
