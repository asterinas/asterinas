// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <sys/sysmacros.h>
#include "../../common/test.h"

#define NULL_DEVICE_PATH "/ext2/my_null_device"
#define ZERO_DEVICE_PATH "/ext2/my_zero_device"
#define FIFO_PATH "/ext2/myfifo.fifo"
#define EXISTING_FILE_PATH "/ext2/mknod_existing_file"
#define EXISTING_SYMLINK_PATH "/ext2/mknod_existing_symlink"

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

FN_TEST(mknod_on_existing_paths_returns_eexist)
{
	int fd = TEST_SUCC(open(EXISTING_FILE_PATH, O_CREAT | O_RDWR, 0666));
	TEST_SUCC(close(fd));
	TEST_SUCC(symlink(EXISTING_FILE_PATH, EXISTING_SYMLINK_PATH));

	TEST_ERRNO(mknod(EXISTING_FILE_PATH, S_IFIFO, 0), EEXIST);
	TEST_ERRNO(mknod(EXISTING_SYMLINK_PATH, S_IFIFO, 0), EEXIST);

	TEST_SUCC(unlink(EXISTING_SYMLINK_PATH));
	TEST_SUCC(unlink(EXISTING_FILE_PATH));
}
END_TEST()
