// SPDX-License-Identifier: MPL-2.0

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "../test.h"

#define INPUT_DIR "/dev/input"
#define MAX_EVDEV_DEVICES 16

static int evdev_fds[MAX_EVDEV_DEVICES];
static size_t evdev_count = 0;

static void cleanup_open_fds(DIR *dir)
{
	if (dir) {
		closedir(dir);
	}
	for (size_t i = 0; i < evdev_count; ++i) {
		close(evdev_fds[i]);
	}
}

FN_SETUP(open_evdev)
{
	DIR *dir = opendir(INPUT_DIR);
	if (!dir) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr, "evdev tests skipped: %s (%s)\n",
				INPUT_DIR, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: setup_open_evdev: opendir('%s') failed: %s\n",
			INPUT_DIR, strerror(errno));
		exit(EXIT_FAILURE);
	}

	struct dirent *entry;
	while ((entry = readdir(dir)) != NULL) {
		if (strncmp(entry->d_name, "event", 5) != 0) {
			continue;
		}

		if (evdev_count >= MAX_EVDEV_DEVICES) {
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: too many event devices (max %d)\n",
				MAX_EVDEV_DEVICES);
			exit(EXIT_FAILURE);
		}

		char path[PATH_MAX];
		int len = snprintf(path, sizeof(path), "%s/%s", INPUT_DIR,
				   entry->d_name);
		if (len < 0 || len >= (int)sizeof(path)) {
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: path too long\n");
			exit(EXIT_FAILURE);
		}

		int fd = open(path, O_RDONLY);
		if (fd < 0) {
			int saved = errno;
			cleanup_open_fds(dir);
			fprintf(stderr,
				"fatal error: setup_open_evdev: open('%s') failed: %s\n",
				path, strerror(saved));
			exit(EXIT_FAILURE);
		}

		evdev_fds[evdev_count++] = fd;
	}

	closedir(dir);

	if (evdev_count == 0) {
		fprintf(stderr,
			"evdev tests skipped: no event devices found in %s\n",
			INPUT_DIR);
		exit(EXIT_SUCCESS);
	}
}
END_SETUP()

// TODO: Currently this test is meaningless.
// It is a placeholder for future tests (e.g., for ioctl commands).
FN_TEST(open_close)
{
	TEST_RES(evdev_count > 0, _ret == 1);
	for (size_t i = 0; i < evdev_count; ++i) {
		TEST_RES(evdev_fds[i] >= 0, _ret == 1);
	}
}
END_TEST()

FN_SETUP(close_evdev)
{
	for (size_t i = 0; i < evdev_count; ++i) {
		CHECK(close(evdev_fds[i]));
	}
}
END_SETUP()
