// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include "../../common/test.h"
#include "../../common/capability.h"

#define NULL_DEVICE_PATH "/ext2/my_null_device"
#define ZERO_DEVICE_PATH "/ext2/my_zero_device"
#define FIFO_PATH "/ext2/myfifo.fifo"
#define NO_CAP_CHAR_DEVICE_PATH "/ext2/no_cap_char_device"
#define NO_CAP_BLOCK_DEVICE_PATH "/ext2/no_cap_block_device"
#define NO_CAP_EXISTING_PATH "/ext2/no_cap_existing"
#define NO_CAP_FIFO_PATH "/ext2/no_cap_fifo"
#define SOCKET_PATH "/ext2/mknod.socket"

FN_TEST(make_device_node)
{
	char buffer[1] = { 'a' };

	TEST_SUCC(mknod(NULL_DEVICE_PATH, S_IFCHR | 0666, makedev(1, 3)));
	int null_fd = TEST_SUCC(open(NULL_DEVICE_PATH, O_RDWR));
	TEST_RES(write(null_fd, buffer, sizeof(buffer)),
		 _ret == sizeof(buffer));
	TEST_RES(read(null_fd, buffer, sizeof(buffer)), _ret == 0);
	TEST_SUCC(close(null_fd));
	TEST_SUCC(unlink(NULL_DEVICE_PATH));

	TEST_SUCC(mknod(ZERO_DEVICE_PATH, S_IFCHR | 0666, makedev(1, 5)));
	int zero_fd = TEST_SUCC(open(ZERO_DEVICE_PATH, O_RDWR));
	TEST_RES(write(zero_fd, buffer, sizeof(buffer)),
		 _ret == sizeof(buffer));
	TEST_RES(read(zero_fd, buffer, sizeof(buffer)),
		 _ret == sizeof(buffer) && buffer[0] == 0);
	TEST_SUCC(close(zero_fd));
	TEST_SUCC(unlink(ZERO_DEVICE_PATH));
}
END_TEST()

FN_TEST(make_device_node_requires_cap_mknod)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(unlink(NO_CAP_EXISTING_PATH),
			   _ret == 0 || errno == ENOENT);
		int existing_fd = CHECK(open(NO_CAP_EXISTING_PATH,
					     O_CREAT | O_EXCL | O_RDWR, 0666));
		CHECK(close(existing_fd));

		drop_capability(CAP_MKNOD);

		CHECK_WITH(mknod(NO_CAP_EXISTING_PATH, S_IFCHR | 0666,
				 makedev(1, 3)),
			   _ret == -1 && errno == EEXIST);
		CHECK_WITH(mknod(NO_CAP_CHAR_DEVICE_PATH, S_IFCHR | 0666,
				 makedev(1, 3)),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(mknod(NO_CAP_BLOCK_DEVICE_PATH, S_IFBLK | 0666,
				 makedev(8, 0)),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(unlink(NO_CAP_FIFO_PATH),
			   _ret == 0 || errno == ENOENT);
		CHECK(mkfifo(NO_CAP_FIFO_PATH, 0666));
		CHECK(unlink(NO_CAP_FIFO_PATH));
		CHECK(unlink(NO_CAP_EXISTING_PATH));

		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(make_fifo_node)
{
	char write_buffer[2] = { 'a', 'b' };
	char read_buffer[1] = { 0 };

	TEST_SUCC(mkfifo(FIFO_PATH, 0666));

	TEST_ERRNO(open(FIFO_PATH, O_WRONLY | O_NONBLOCK), ENXIO);
	int reader_fd = TEST_SUCC(open(FIFO_PATH, O_RDONLY | O_NONBLOCK));
	int writer_fd = TEST_SUCC(open(FIFO_PATH, O_WRONLY));

	TEST_RES(write(writer_fd, write_buffer, sizeof(write_buffer)),
		 _ret == sizeof(write_buffer));
	TEST_RES(read(reader_fd, read_buffer, sizeof(read_buffer)),
		 _ret == sizeof(read_buffer) && read_buffer[0] == 'a');
	TEST_RES(read(reader_fd, read_buffer, sizeof(read_buffer)),
		 _ret == sizeof(read_buffer) && read_buffer[0] == 'b');

	TEST_SUCC(close(reader_fd));
	TEST_SUCC(close(writer_fd));
	TEST_SUCC(unlink(FIFO_PATH));
}
END_TEST()

FN_TEST(make_socket_node)
{
	struct stat statbuf;

	TEST_SUCC(mknod(SOCKET_PATH, S_IFSOCK | 0600, 0));
	TEST_RES(lstat(SOCKET_PATH, &statbuf),
		 _ret == 0 && S_ISSOCK(statbuf.st_mode));
	TEST_SUCC(unlink(SOCKET_PATH));
}
END_TEST()
