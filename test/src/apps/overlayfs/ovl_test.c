// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/mount.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>

#define OVERLAYDIR "/overlay"
#define LOWERDIR1 OVERLAYDIR "/lower1"
#define LOWERDIR2 OVERLAYDIR "/lower2"
#define UPPERDIR OVERLAYDIR "/upper"
#define WORKDIR OVERLAYDIR "/work"
#define MERGEDDIR OVERLAYDIR "/merged"

void clean_up();

void handle_error(const char *msg)
{
	perror(msg);
	// Perform cleanup before exit to ensure no leftover directories or mount points
	clean_up();
	exit(EXIT_FAILURE);
}

void create_dir(const char *path)
{
	printf("Creating directory: %s\n", path);
	if (mkdir(path, 0755) == -1 && errno != EEXIST) {
		handle_error("mkdir");
	}
}

void write_to_file(const char *path, const char *data)
{
	printf("Writing to file: %s\n", path);
	int fd = open(path, O_WRONLY | O_CREAT, 0644);
	if (fd == -1) {
		handle_error("open");
	}
	if (write(fd, data, strlen(data)) == -1) {
		close(fd);
		handle_error("write");
	}
	close(fd);
}

void read_file(const char *path, char *buffer, size_t size)
{
	int fd = open(path, O_RDONLY);
	if (fd == -1) {
		handle_error("open");
	}
	ssize_t bytesRead = read(fd, buffer, size);
	if (bytesRead == -1) {
		close(fd);
		handle_error("read");
	}
	buffer[bytesRead] = '\0'; // Null-terminate the string
	close(fd);
}

void write_data_at_offset(const char *path, const char *data, off_t offset)
{
	int fd = open(path, O_WRONLY);
	if (fd == -1) {
		handle_error("open");
	}
	if (lseek(fd, offset, SEEK_SET) == -1) {
		close(fd);
		handle_error("lseek");
	}
	if (write(fd, data, strlen(data)) == -1) {
		close(fd);
		handle_error("write");
	}
	close(fd);
}

void assert_eq(const char *result, const char *expected, const char *msg)
{
	if (strcmp(result, expected) != 0) {
		fprintf(stderr, "Assertion failed: %s\nExpected: %s\nGot: %s\n",
			msg, expected, result);
		handle_error("assert_eq");
	}
}

void mount_overlayfs()
{
	char options[512];
	snprintf(options, sizeof(options),
		 "lowerdir=%s:%s,upperdir=%s,workdir=%s", LOWERDIR1, LOWERDIR2,
		 UPPERDIR, WORKDIR);
	printf("Mount options: %s\n", options);

	if (mount("overlay", MERGEDDIR, "overlay", 0, options) == -1) {
		handle_error("mount overlay");
	}
}

void unlink_if_exists(const char *path)
{
	if (unlink(path) == -1 && errno != ENOENT) {
		handle_error("unlink");
	}
}

void clean_up()
{
	umount(MERGEDDIR);
	umount(OVERLAYDIR);
	rmdir(MERGEDDIR);
	rmdir(WORKDIR);
	rmdir(UPPERDIR);
	rmdir(LOWERDIR1);
	rmdir(LOWERDIR2);
	rmdir(OVERLAYDIR);
}

// TODO: Enrich this test
int main()
{
	create_dir(OVERLAYDIR);
	// Mount tmpfs to /overlay first (Linux only)
	// if (mount("tmpfs", OVERLAYDIR, "tmpfs", 0, NULL) == -1) {
	//     handle_error("mount tmpfs");
	// }

	// Create necessary directories now that tmpfs is mounted
	create_dir(LOWERDIR1);
	create_dir(LOWERDIR2);
	create_dir(UPPERDIR);
	create_dir(WORKDIR);
	create_dir(MERGEDDIR);

	// Create files and directories in lower directories
	create_dir(LOWERDIR1 "/d1");
	write_to_file(LOWERDIR1 "/d1/f11", "file in lower1");

	write_to_file(LOWERDIR2 "/f2", "88");
	create_dir(LOWERDIR2 "/d1");
	write_to_file(LOWERDIR2 "/d1/f11", "another file in lower2 d1");
	write_to_file(LOWERDIR2 "/d1/f12", "another file in lower2 d1 f12");

	printf("Mounting OverlayFS\n");
	// Mount OverlayFS
	mount_overlayfs();

	// Read and verify the contents of the merged directory
	char buffer[1024];

	printf("Reading /overlay/merged/f2:\n");
	read_file(MERGEDDIR "/f2", buffer, sizeof(buffer));
	assert_eq(buffer, "88", "Content of /overlay/merged/f2 should be '88'");

	// Write and read test - Copy up test
	printf("Writing '99' to /overlay/merged/f2 at offset 2\n");
	write_data_at_offset(MERGEDDIR "/f2", "99", 2);

	read_file(MERGEDDIR "/f2", buffer, sizeof(buffer));
	assert_eq(
		buffer, "8899",
		"Content of /overlay/merged/f2 should be '8899' after writing '99' at offset 2");

	// Whiteout test
	printf("Unlinking /overlay/merged/f1 if exists\n");
	unlink_if_exists(MERGEDDIR "/f1");

	// Create a new f1
	write_to_file(MERGEDDIR "/f1", "new content for f1");

	// Read and verify the new f1
	printf("Reading /overlay/merged/f1:\n");
	read_file(MERGEDDIR "/f1", buffer, sizeof(buffer));
	assert_eq(
		buffer, "new content for f1",
		"Content of /overlay/merged/f1 should be 'new content for f1'");

	// Clean up before exit
	printf("Cleaning up\n");
	clean_up();

	return 0;
}
