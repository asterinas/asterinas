// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <mqueue.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define MQUEUE_DIR "/dev/mqueue"
#define QUEUE_NAME "/test_queue"

FN_SETUP(setup_mqueuefs)
{
	CHECK_WITH(mkdir(MQUEUE_DIR, 0755), _ret == 0 || errno == EEXIST);
	CHECK(mount("mqueue", MQUEUE_DIR, "mqueue", 0, NULL));
}
END_SETUP()

FN_TEST(mqueuefs_mount_point_is_directory)
{
	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR, &st));
	TEST_RES(S_ISDIR(st.st_mode), _ret == 1);
}
END_TEST()

FN_TEST(mq_open_creates_a_queue)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_reopens_existing_queue)
{
	mqd_t mq1 = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq1));

	mqd_t mq2 = TEST_SUCC(mq_open(QUEUE_NAME, O_RDWR));
	TEST_SUCC(mq_close(mq2));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_missing_queue_fails_with_enoent)
{
	TEST_ERRNO(mq_open("/no_such_queue", O_RDONLY), ENOENT);
}
END_TEST()

FN_TEST(mq_open_excl_fails_with_eexist)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_ERRNO(mq_open(QUEUE_NAME, O_CREAT | O_EXCL | O_RDWR, 0644, NULL),
		   EEXIST);
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_empty_name_fails_with_einval)
{
	TEST_ERRNO(mq_open("", O_CREAT | O_RDWR, 0644, NULL), EINVAL);
}
END_TEST()

FN_TEST(mq_open_name_with_slash_fails_with_einval)
{
	TEST_ERRNO(mq_open("/a/b", O_CREAT | O_RDWR, 0644, NULL), EINVAL);
}
END_TEST()

FN_TEST(mq_unlink_missing_queue_fails_with_enoent)
{
	TEST_ERRNO(mq_unlink("/no_such_queue"), ENOENT);
}
END_TEST()

FN_TEST(mq_unlink_removes_queue)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
	TEST_ERRNO(mq_open(QUEUE_NAME, O_RDONLY), ENOENT);
}
END_TEST()

FN_TEST(mqueuefs_stat_queue_file)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR QUEUE_NAME, &st));
	TEST_RES(S_ISREG(st.st_mode), _ret == 1);
	TEST_RES(st.st_uid == getuid(), _ret == 1);
	TEST_RES(st.st_gid == getgid(), _ret == 1);
	TEST_RES(st.st_size == 0, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mqueuefs_stat_missing_queue_fails_with_enoent)
{
	struct stat st;
	TEST_ERRNO(stat(MQUEUE_DIR "/no_such_queue", &st), ENOENT);
}
END_TEST()

FN_TEST(mqueuefs_stat_after_unlink_fails_with_enoent)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));

	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR QUEUE_NAME, &st));

	TEST_SUCC(mq_unlink(QUEUE_NAME));
	TEST_ERRNO(stat(MQUEUE_DIR QUEUE_NAME, &st), ENOENT);
}
END_TEST()
