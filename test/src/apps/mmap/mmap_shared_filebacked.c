// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>

#define FILE_PATH "test.dat"
#define FILE_SIZE 4096
#define CONTENT "shared filebacked mmap test!"

void create_file(const char *path, size_t size)
{
	int fd = open(path, O_RDWR | O_CREAT, 0666);
	if (fd == -1) {
		perror("open");
		exit(EXIT_FAILURE);
	}

	// Ensure the file is of the required size
	if (ftruncate(fd, size) == -1) {
		perror("ftruncate");
		close(fd);
		exit(EXIT_FAILURE);
	}

	close(fd);
}

void verify_file_contents(const char *path, const char *expected_content)
{
	FILE *file = fopen(path, "r");
	if (file == NULL) {
		perror("fopen");
		exit(EXIT_FAILURE);
	}

	char buffer[FILE_SIZE] = { 0 };
	fread(buffer, 1, FILE_SIZE, file);
	fclose(file);

	if (strstr(buffer, expected_content) != NULL) {
		printf("Content written to the file successfully matches the expected content.\n");
	} else {
		printf("Content mismatch! Expected content was not found in the file.\n");
	}
}

int main()
{
	// Create a file
	create_file(FILE_PATH, FILE_SIZE);

	// Open the file
	int fd = open(FILE_PATH, O_RDWR);
	if (fd == -1) {
		perror("open");
		exit(EXIT_FAILURE);
	}

	// Create a shared mmap
	char *map = (char *)mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE,
				 MAP_SHARED, fd, 0);
	if (map == MAP_FAILED) {
		perror("mmap");
		close(fd);
		exit(EXIT_FAILURE);
	}

	// Write to the memory
	strcpy(map, CONTENT);

	// Unmap the memory
	if (munmap(map, FILE_SIZE) == -1) {
		perror("munmap");
	}

	// Close file descriptor
	close(fd);

	// Verify the file contents
	verify_file_contents(FILE_PATH, CONTENT);

	// Remove the file before exiting
	if (remove(FILE_PATH) != 0) {
		perror("remove");
	} else {
		printf("Temporary file removed successfully.\n");
	}

	return 0;
}
