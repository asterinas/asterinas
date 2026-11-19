// SPDX-License-Identifier: MPL-2.0

/*
 * Linux O_NONBLOCK/workqueue regression tests for /dev/tpm0.
 *
 * O_NONBLOCK moves command execution to background work. Command validation
 * remains synchronous in write; poll/read later delivers the result or error.
 */

#include "tpm_test_common.h"

FN_SETUP(check_tpm_availability)
{
	tpm_require_device(TPM_DEVICE);
}
END_SETUP()

/* Cover async write, duplicate-write EBUSY, POLLIN, and response delivery. */
FN_TEST(tpm_nonblocking_write_poll_read)
{
	uint8_t response[256] = { 0 };
	struct pollfd pfd;
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR | O_NONBLOCK));

	TEST_RES(read(fd, response, sizeof(response)), _ret == 0);

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	/* command_enqueued or a completely unread response makes write EBUSY. */
	TEST_ERRNO(write(fd, tpm_get_random_command,
			 sizeof(tpm_get_random_command)),
		   EBUSY);

	len = TEST_RES(tpm_wait_and_read_response(fd, response,
						  sizeof(response), 5000),
		       _ret >= TPM_HEADER_SIZE + 2);

	if (len >= TPM_HEADER_SIZE + 2) {
		TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)len);
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};
	TEST_RES(poll(&pfd, 1, 0), _ret == 1 && (pfd.revents & POLLOUT) &&
					   !(pfd.revents & POLLIN));

	TEST_SUCC(close(fd));
}
END_TEST()

/* O_NONBLOCK is mutable open-file state and must also work when set by fcntl. */
FN_TEST(tpm_dynamic_nonblocking_via_fcntl)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_SUCC(fcntl(fd, F_SETFL, O_NONBLOCK));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	len = TEST_RES(tpm_wait_and_read_response(fd, response,
						  sizeof(response), 5000),
		       _ret >= TPM_HEADER_SIZE + 2);

	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* A TPM protocol error remains a readable response, not a write errno. */
FN_TEST(tpm_nonblocking_unknown_command_returns_response)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR | O_NONBLOCK));

	TEST_RES(write(fd, tpm_unknown_command, sizeof(tpm_unknown_command)),
		 _ret == sizeof(tpm_unknown_command));

	len = TEST_RES(tpm_wait_and_read_response(fd, response,
						  sizeof(response), 5000),
		       _ret >= TPM_HEADER_SIZE);

	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* An inconsistent header length must return EINVAL before work is queued. */
FN_TEST(tpm_nonblocking_header_length_error_is_synchronous)
{
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR | O_NONBLOCK));

	/* tpm_common_write validates this before queue_work(). */
	TEST_ERRNO(write(fd, tpm_invalid_size_command,
			 sizeof(tpm_invalid_size_command)),
		   EINVAL);

	TEST_SUCC(close(fd));
}
END_TEST()

/* The Linux char layer accepts this short buffer; transport fails later. */
FN_TEST(tpm_nonblocking_nine_byte_buffer_is_accepted_by_char_layer)
{
	uint8_t command[9] = { 0 };
	uint8_t response[64] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR | O_NONBLOCK));

	/* Linux tpm_common_write only checks size >= 6 and header length. */
	TEST_RES(write(fd, command, sizeof(command)), _ret == sizeof(command));

	/* The later async transport failure is read back after poll(POLLIN). */
	len = tpm_wait_and_read_response(fd, response, sizeof(response), 5000);
	TEST_RES(len, _ret < 0 || _ret >= TPM_HEADER_SIZE);

	TEST_SUCC(close(fd));
}
END_TEST()
