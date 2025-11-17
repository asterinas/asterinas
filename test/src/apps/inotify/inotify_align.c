// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

#define INOTIFY_EVENT_SIZE 16 // sizeof(struct inotify_event)

static void die(const char *msg)
{
	perror(msg);
	exit(1);
}

static void ensure_dir(const char *path)
{
	if (mkdir(path, 0700) < 0 && errno != EEXIST)
		die("mkdir");
}

static void create_file(const char *path)
{
	int fd = open(path, O_CREAT | O_WRONLY, 0600);
	if (fd < 0)
		die("open");
	close(fd);
}

// Round up to the nearest multiple of 16
static size_t round_to_16(size_t n)
{
	return (n + 15) & ~15;
}

// Calculate expected event size based on name length
static size_t expected_event_size(const char *name)
{
	size_t name_len = name ? strlen(name) + 1 : 0; // +1 for null terminator
	size_t header_size = INOTIFY_EVENT_SIZE;
	size_t padded_name_len = round_to_16(name_len);
	return header_size + padded_name_len;
}

int main(void)
{
	const char *dir = "inotify_align_tmp";

	// Test files with different name lengths to test alignment
	const char *test_files[] = {
		"inotify_align_tmp/a", // 2 bytes name (1 char + null)
		"inotify_align_tmp/ab", // 3 bytes name
		"inotify_align_tmp/abc", // 4 bytes name
		"inotify_align_tmp/abcd", // 5 bytes name
		"inotify_align_tmp/abcdefgh", // 9 bytes name
		"inotify_align_tmp/abcdefghijklmnop", // 17 bytes name
		NULL
	};

	ensure_dir(dir);

	int ifd = inotify_init1(0);
	if (ifd < 0)
		die("inotify_init1");

	int wd = inotify_add_watch(ifd, dir, IN_CREATE);
	if (wd < 0)
		die("inotify_add_watch");

	// Create test files to generate events
	for (int i = 0; test_files[i] != NULL; i++) {
		create_file(test_files[i]);
	}

	// Wait a bit for events to be queued
	struct timespec ts = { .tv_sec = 0, .tv_nsec = 100 * 1000 * 1000 };
	nanosleep(&ts, NULL);

	// Check FIONREAD before reading
	int pending = 0;
	if (ioctl(ifd, FIONREAD, &pending) < 0)
		die("ioctl(FIONREAD)");

	printf("FIONREAD reports %d bytes pending\n", pending);

	// Read all events
	char buf[4096]
		__attribute__((aligned(__alignof__(struct inotify_event))));
	ssize_t total_read = read(ifd, buf, sizeof(buf));
	if (total_read < 0)
		die("read");

	printf("Read %zd bytes total\n", total_read);

	// Verify FIONREAD matches actual read size
	if (total_read != pending) {
		fprintf(stderr,
			"ERROR: FIONREAD (%d) != actual read size (%zd)\n",
			pending, total_read);
		return 1;
	}

	// Parse and verify events
	size_t offset = 0;
	int event_count = 0;
	const char *expected_names[] = { "a",	 "ab",	     "abc",
					 "abcd", "abcdefgh", "abcdefghijklmnop",
					 NULL };

	while (offset < (size_t)total_read) {
		if (offset + INOTIFY_EVENT_SIZE > (size_t)total_read) {
			fprintf(stderr,
				"ERROR: Incomplete event at offset %zu\n",
				offset);
			return 1;
		}

		struct inotify_event *event =
			(struct inotify_event *)(buf + offset);

		// Calculate expected size for this event
		const char *expected_name = expected_names[event_count];

		// Verify event is aligned
		if ((uintptr_t)event % __alignof__(struct inotify_event) != 0) {
			fprintf(stderr,
				"ERROR: Event at offset %zu is not aligned\n",
				offset);
			return 1;
		}

		// Verify len field matches expected padded length
		size_t actual_name_len =
			expected_name ? strlen(expected_name) + 1 : 0;
		size_t expected_padded_len = round_to_16(actual_name_len);

		if (event->len != expected_padded_len) {
			fprintf(stderr,
				"ERROR: Event %d: len field is %u, expected %zu (name_len=%zu)\n",
				event_count, event->len, expected_padded_len,
				actual_name_len);
			return 1;
		}

		// Verify event size is aligned to 16 bytes
		size_t event_size = INOTIFY_EVENT_SIZE + event->len;
		size_t expected_size = expected_event_size(expected_name);
		if (event_size != expected_size) {
			fprintf(stderr,
				"ERROR: Event %d: total size %zu doesn't match expected %zu\n",
				event_count, event_size, expected_size);
			return 1;
		}
		if (event_size % INOTIFY_EVENT_SIZE != 0) {
			fprintf(stderr,
				"ERROR: Event %d: total size %zu is not aligned to %d bytes\n",
				event_count, event_size, INOTIFY_EVENT_SIZE);
			return 1;
		}

		// Verify name if present
		if (expected_name && event->len > 0) {
			if (strcmp(event->name, expected_name) != 0) {
				fprintf(stderr,
					"ERROR: Event %d: name mismatch: got '%s', expected '%s'\n",
					event_count, event->name,
					expected_name);
				return 1;
			}

			// Verify padding bytes are zero
			size_t actual_name_len_with_null =
				strlen(expected_name) + 1;
			if (event->len > actual_name_len_with_null) {
				size_t padding_start =
					actual_name_len_with_null;
				for (size_t i = padding_start; i < event->len;
				     i++) {
					if (event->name[i] != '\0') {
						fprintf(stderr,
							"ERROR: Event %d: padding byte at offset %zu is not zero (0x%02x)\n",
							event_count, i,
							(unsigned char)
								event->name[i]);
						return 1;
					}
				}
			}
		}

		printf("Event %d: wd=%d, mask=0x%x, len=%u, name='%s', size=%zu (aligned: %s)\n",
		       event_count, event->wd, event->mask, event->len,
		       event->len > 0 ? event->name : "(none)", event_size,
		       (event_size % INOTIFY_EVENT_SIZE == 0) ? "yes" : "no");

		// Move to next event
		offset += event_size;
		event_count++;

		// Verify next event offset is aligned
		if (offset < (size_t)total_read &&
		    offset % INOTIFY_EVENT_SIZE != 0) {
			fprintf(stderr,
				"ERROR: Next event offset %zu is not aligned to %d bytes\n",
				offset, INOTIFY_EVENT_SIZE);
			return 1;
		}
	}

	// Cleanup
	close(ifd);
	for (int i = 0; test_files[i] != NULL; i++) {
		unlink(test_files[i]);
	}
	rmdir(dir);

	printf("\nAll alignment tests passed! Processed %d events.\n",
	       event_count);
	printf("Summary:\n");
	printf("  - FIONREAD matched actual read size: ✓\n");
	printf("  - All events aligned to 16 bytes: ✓\n");
	printf("  - All len fields correct (padded): ✓\n");
	printf("  - All padding bytes are zero: ✓\n");
	printf("  - Event offsets are aligned: ✓\n");

	return 0;
}
