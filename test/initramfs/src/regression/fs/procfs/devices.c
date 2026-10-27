// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <assert.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../../common/test.h"

#define DEVICES_PATH "/proc/devices"
#define BUFFER_SIZE 16384

static size_t read_devices(char *buffer, size_t chunk)
{
	int fd = open(DEVICES_PATH, O_RDONLY);
	assert(fd >= 0);
	size_t total = 0;
	for (;;) {
		assert(total < BUFFER_SIZE - 1);
		size_t capacity = BUFFER_SIZE - 1 - total;
		size_t size = chunk < capacity ? chunk : capacity;
		ssize_t count = read(fd, buffer + total, size);
		assert(count >= 0);
		if (!count)
			break;
		total += count;
	}
	buffer[total] = '\0';
	int result = close(fd);
	assert(result == 0);
	return total;
}

FN_TEST(devices_sections_and_registered_names)
{
	char buffer[BUFFER_SIZE];
	read_devices(buffer, BUFFER_SIZE);
	char *save = NULL;
	char *line = strtok_r(buffer, "\n", &save);
	TEST_RES(line != NULL, _ret == 1);
	TEST_RES(strcmp(line, "Character devices:") == 0, _ret == 1);

	int section = 0;
	unsigned int previous = 0;
	int have_previous = 0;
	int mem = 0, misc = 0, input = 0, tty = 0, console = 0;
	int ptmx = 0, pts = 0, blkext = 0;
	while ((line = strtok_r(NULL, "\n", &save)) != NULL) {
		if (strcmp(line, "Block devices:") == 0) {
			TEST_RES(section == 0, _ret == 1);
			section = 1;
			have_previous = 0;
			continue;
		}
		unsigned int major_id;
		char name[128], extra;
		TEST_RES(sscanf(line, "%u %127s %c", &major_id, name, &extra),
			 _ret == 2);
		TEST_RES(major_id <= 511, _ret == 1);
		if (have_previous)
			TEST_RES(major_id >= previous, _ret == 1);
		previous = major_id;
		have_previous = 1;
		if (section == 0) {
			mem += major_id == 1 && strcmp(name, "mem") == 0;
			misc += major_id == 10 && strcmp(name, "misc") == 0;
			input += major_id == 13 && strcmp(name, "input") == 0;
			tty += major_id == 5 && strcmp(name, "/dev/tty") == 0;
			console += major_id == 5 &&
				   strcmp(name, "/dev/console") == 0;
			ptmx += major_id == 5 && strcmp(name, "/dev/ptmx") == 0;
			pts += major_id == 136 && strcmp(name, "pts") == 0;
		} else {
			blkext += major_id == 259 &&
				  strcmp(name, "blkext") == 0;
		}
	}
	TEST_RES(section == 1, _ret == 1);
	TEST_RES(mem == 1 && misc == 1 && input == 1, _ret == 1);
	TEST_RES(tty == 1 && console == 1 && ptmx == 1 && pts == 1, _ret == 1);
	TEST_RES(blkext == 1, _ret == 1);

	/* The control node belongs to misc; it is not a separate major row. */
	struct stat st;
	TEST_SUCC(stat("/dev/mapper/control", &st));
	TEST_RES(S_ISCHR(st.st_mode), _ret != 0);
	TEST_RES(major(st.st_rdev) == 10 && minor(st.st_rdev) == 236,
		 _ret == 1);
}
END_TEST()

FN_TEST(devices_partial_reads_seek_and_eof)
{
	char full[BUFFER_SIZE], chunks[BUFFER_SIZE];
	size_t length = read_devices(full, BUFFER_SIZE);
	TEST_RES(read_devices(chunks, 7), _ret == length);
	TEST_RES(memcmp(full, chunks, length), _ret == 0);

	int fd = TEST_SUCC(open(DEVICES_PATH, O_RDONLY));
	TEST_SUCC(lseek(fd, 3, SEEK_SET));
	ssize_t count = TEST_SUCC(read(fd, chunks, 11));
	TEST_RES(count == 11, _ret == 1);
	TEST_RES(memcmp(chunks, full + 3, count), _ret == 0);
	TEST_SUCC(lseek(fd, length, SEEK_SET));
	TEST_RES(read(fd, chunks, sizeof(chunks)), _ret == 0);
	TEST_SUCC(lseek(fd, length + 100, SEEK_SET));
	TEST_RES(read(fd, chunks, sizeof(chunks)), _ret == 0);
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_RES(read(fd, chunks, 0), _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()
