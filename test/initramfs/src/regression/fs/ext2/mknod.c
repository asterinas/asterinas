// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <sys/sysmacros.h>
#include "../../common/test.h"

#define NULL_DEVICE_PATH "/ext2/my_null_device"
#define ZERO_DEVICE_PATH "/ext2/my_zero_device"
#define FIFO_PATH "/ext2/myfifo.fifo"

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