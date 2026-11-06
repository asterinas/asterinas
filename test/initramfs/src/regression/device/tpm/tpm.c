// SPDX-License-Identifier: MPL-2.0

/*
 * Linux-compatible character-device ABI regression tests for raw /dev/tpm0.
 *
 * This file covers user-visible file semantics: device identity, exclusive
 * open, access modes, write/read/poll state, and close behavior. TPM command
 * behavior lives in tpm_protocol.c; resource-manager behavior lives in tpmrm.c.
 */

#include <pthread.h>
#include <sys/ioctl.h>

#include "tpm_test_common.h"

struct shared_writer_args {
	int fd;
	ssize_t result;
	int error;
};

/* Share one open file description to expose same-file write races. */
static void *shared_writer_thread(void *arg)
{
	struct shared_writer_args *args = arg;

	errno = 0;
	args->result = write(args->fd, tpm_get_random_command,
			     sizeof(tpm_get_random_command));
	args->error = errno;
	return NULL;
}

FN_SETUP(check_tpm_availability)
{
	tpm_require_device(TPM_DEVICE);
}
END_SETUP()

/* Verify the Linux misc-device identity assigned to /dev/tpm0: 10:224. */
FN_TEST(tpm_device_identity)
{
	struct stat stat_buf;

	/*
	 * Do not assert mode 0600: userspace/udev may legitimately change
	 * the mode. The Linux raw TPM device identity is major 10, minor 224.
	 */
	TEST_RES(stat(TPM_DEVICE, &stat_buf),
		 S_ISCHR(stat_buf.st_mode) &&
			 stat_buf.st_rdev == makedev(TPM_MAJOR, TPM_MINOR));
}
END_TEST()

/* The raw device is exclusive; a second independent open must return EBUSY. */
FN_TEST(tpm_raw_device_is_exclusive)
{
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	/* Linux raw /dev/tpm0 allows only one independent open. */
	TEST_ERRNO(open(TPM_DEVICE, O_RDWR), EBUSY);

	TEST_SUCC(close(fd));
}
END_TEST()

/* An idle file is writable, not readable, and not seekable. */
FN_TEST(tpm_idle_file_semantics)
{
	uint8_t byte = 0;
	struct pollfd pfd;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	/* No pending response: Linux reports writable, not readable. */
	TEST_RES(poll(&pfd, 1, 0), _ret == 1 && (pfd.revents & POLLOUT) &&
					   !(pfd.revents & POLLIN));

	TEST_RES(read(fd, &byte, 0), _ret == 0);

	/* Linux tpm_common_read() returns 0 when no response is pending. */
	TEST_RES(read(fd, &byte, 1), _ret == 0);

	TEST_ERRNO(lseek(fd, 0, SEEK_SET), ESPIPE);

	TEST_SUCC(close(fd));
}
END_TEST()

/* VFS access-mode checks must return EBADF before TPM command handling. */
FN_TEST(tpm_access_modes_are_enforced)
{
	uint8_t byte = 0;
	int fd;

	fd = TEST_SUCC(open(TPM_DEVICE, O_RDONLY));
	TEST_ERRNO(write(fd, tpm_get_random_command,
			 sizeof(tpm_get_random_command)),
		   EBADF);
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(TPM_DEVICE, O_WRONLY));
	TEST_ERRNO(read(fd, &byte, sizeof(byte)), EBADF);
	TEST_SUCC(close(fd));
}
END_TEST()

/* Cover the minimum size, declared header size, and 4096-byte ABI limit. */
FN_TEST(tpm_rejects_invalid_command_sizes)
{
	uint8_t too_short[5] = { 0 };
	uint8_t declared_too_long[10] = {
		0x80, 0x01, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x01, 0x7b,
	};
	uint8_t *too_large = TEST_RES(calloc(1, TPM_BUFSIZE + 1), _ret != NULL);
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	/* Linux tpm_common_write(): size < 6 -> EINVAL. */
	TEST_ERRNO(write(fd, too_short, sizeof(too_short)), EINVAL);

	/*
	 * The TPM header declares 12 bytes while write() supplies 10:
	 * Linux rejects this in the character-device layer with EINVAL.
	 */
	TEST_ERRNO(write(fd, declared_too_long, sizeof(declared_too_long)),
		   EINVAL);

	/* Linux TPM_BUFSIZE is 4096; larger writes return E2BIG. */
	TEST_ERRNO(write(fd, too_large, TPM_BUFSIZE + 1), E2BIG);

	TEST_SUCC(close(fd));
	free(too_large);
}
END_TEST()

/* A zero-length write and an unknown ioctl are character-device ABI errors. */
FN_TEST(tpm_zero_length_write_and_unknown_ioctl)
{
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_ERRNO(write(fd, tpm_get_random_command, 0), EINVAL);
	TEST_ERRNO(ioctl(fd, 0, NULL), ENOTTY);

	TEST_SUCC(close(fd));
}
END_TEST()

/* The TPM reports an invalid tag in its response; the char layer accepts it. */
FN_TEST(tpm_invalid_tag_is_returned_as_tpm_error)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	/*
	 * The raw character layer does not reject the tag itself. The command
	 * reaches the TPM and the TPM reports the protocol error in response.
	 */
	TEST_RES(write(fd, tpm_invalid_tag_command,
		       sizeof(tpm_invalid_tag_command)),
		 _ret == sizeof(tpm_invalid_tag_command));

	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE);
	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* A userspace-copy failure must not consume or overwrite the current state. */
FN_TEST(tpm_bad_user_buffer_semantics)
{
	void *bad_address = (void *)(uintptr_t)-1;
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_ERRNO(write(fd, bad_address, TPM_HEADER_SIZE), EFAULT);

	/*
	 * On the tested Linux syscall/VFS path, the invalid destination is
	 * rejected with EFAULT even when there is currently no TPM response.
	 */
	TEST_ERRNO(read(fd, bad_address, 1), EFAULT);

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	/*
	 * The failed userspace copy does not consume the pending response in
	 * the observed Linux baseline, so a new command is still EBUSY.
	 */
	TEST_ERRNO(read(fd, bad_address, 1), EFAULT);
	TEST_ERRNO(write(fd, tpm_get_random_command,
			 sizeof(tpm_get_random_command)),
		   EBUSY);

	/* Consume the original response and verify that the device recovers. */
	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* Only one concurrent writer may enter the pending/response lifecycle. */
FN_TEST(tpm_same_file_serializes_writers)
{
	struct shared_writer_args args[2] = { 0 };
	pthread_t threads[2];
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	for (size_t i = 0; i < 2; i++) {
		args[i].fd = fd;
		TEST_RES(pthread_create(&threads[i], NULL, shared_writer_thread,
					&args[i]),
			 _ret == 0);
	}

	for (size_t i = 0; i < 2; i++)
		TEST_RES(pthread_join(threads[i], NULL), _ret == 0);

	TEST_RES((args[0].result == sizeof(tpm_get_random_command)) +
			 (args[1].result == sizeof(tpm_get_random_command)),
		 _ret == 1);

	TEST_RES((args[0].result == -1 && args[0].error == EBUSY) +
			 (args[1].result == -1 && args[1].error == EBUSY),
		 _ret == 1);

	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* File descriptors created by dup share the response buffer and read offset. */
FN_TEST(tpm_dup_shares_file_state)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));
	int dup_fd = TEST_SUCC(dup(fd));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	/* dup() refers to the same open file description / TPM response state. */
	len = TEST_RES(read(dup_fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	/* The response was consumed through dup_fd. */
	TEST_RES(read(fd, response, sizeof(response)), _ret == 0);

	TEST_SUCC(close(dup_fd));
	TEST_SUCC(close(fd));
}
END_TEST()

/* Closing a raw fd does not clean up sessions already created on the TPM. */
FN_TEST(tpm_raw_close_preserves_session)
{
	uint8_t response[512] = { 0 };
	uint8_t flush_command[14];
	uint32_t session_handle = 0;
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	len = TEST_RES(tpm_transact_sync(fd, tpm_start_auth_session_command,
					 sizeof(tpm_start_auth_session_command),
					 response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 4);

	if (len < TPM_HEADER_SIZE + 4) {
		TEST_SUCC(close(fd));
		return;
	}

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	session_handle = tpm_read_be32(response + TPM_HEADER_SIZE);

	TEST_SUCC(close(fd));

	/*
	 * Raw /dev/tpm0 does not provide RM-style per-open resource cleanup.
	 * The session remains in the TPM after close().
	 */
	fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));
	tpm_build_flush_context_command(flush_command, session_handle);

	len = TEST_RES(tpm_transact_sync(fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret == TPM_HEADER_SIZE);
	if (len == TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	/* A repeated flush reaches the TPM and returns a TPM error response. */
	len = TEST_RES(tpm_transact_sync(fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret >= TPM_HEADER_SIZE);
	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()