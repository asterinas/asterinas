// SPDX-License-Identifier: MPL-2.0

/*
 * Synchronous TPM 2.0 message regression tests through /dev/tpm0.
 *
 * Besides basic command success paths, this file covers response lengths,
 * partial reads, preserved error responses, and ContextSave/ContextLoad.
 * Full key, quote, seal, and policy functionality is intentionally out of scope.
 */

#include "tpm_test_common.h"

FN_SETUP(check_tpm_availability)
{
	tpm_require_device(TPM_DEVICE);
}
END_SETUP()

/* GetRandom covers poll state, partial reads, and the TPM response header. */
FN_TEST(tpm_get_random_and_partial_reads)
{
	uint8_t response[256] = { 0 };
	struct pollfd pfd;
	ssize_t first_len;
	ssize_t rest_len;
	ssize_t response_len;
	uint16_t random_len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, 0), _ret == 1 && (pfd.revents & POLLIN) &&
					   !(pfd.revents & POLLOUT));

	first_len = TEST_RES(read(fd, response, 5), _ret == 5);
	if (first_len != 5) {
		TEST_SUCC(close(fd));
		return;
	}

	rest_len = TEST_RES(read(fd, response + first_len,
				 sizeof(response) - (size_t)first_len),
			    _ret > 0);
	if (rest_len <= 0) {
		TEST_SUCC(close(fd));
		return;
	}

	response_len = first_len + rest_len;
	if (response_len < TPM_HEADER_SIZE + 2) {
		TEST_SUCC(close(fd));
		return;
	}

	random_len = tpm_read_be16(response + TPM_HEADER_SIZE);

	TEST_RES(tpm_read_be16(response), _ret == TPM_ST_NO_SESSIONS);
	TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)response_len);
	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	TEST_RES(random_len, _ret <= 32);
	TEST_RES(response_len, _ret == TPM_HEADER_SIZE + 2 + random_len);

	/* Linux tpm_common_read() returns 0 after the response is exhausted. */
	TEST_RES(read(fd, response, sizeof(response)), _ret == 0);

	pfd.revents = 0;
	TEST_RES(poll(&pfd, 1, 0), _ret == 1 && (pfd.revents & POLLOUT) &&
					   !(pfd.revents & POLLIN));

	TEST_SUCC(close(fd));
}
END_TEST()

/* A completely unread response occupies the file buffer; new writes are busy. */
FN_TEST(tpm_unread_response_blocks_new_command)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	TEST_ERRNO(write(fd, tpm_get_random_command,
			 sizeof(tpm_get_random_command)),
		   EBUSY);

	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* Linux permits a new write after the first successful, possibly partial, read. */
FN_TEST(tpm_partial_read_allows_new_command)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	TEST_RES(read(fd, response, 5), _ret == 5);

	/* response_read=true after any successful read, so Linux allows this. */
	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	memset(response, 0, sizeof(response));
	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);

	if (len >= TPM_HEADER_SIZE + 2) {
		TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)len);
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

/* On Linux, a zero-length read ends the current response lifecycle. */
FN_TEST(tpm_zero_length_read_discards_pending_response)
{
	uint8_t response[256] = { 0 };
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));

	/* min(0, response_length)==0 and Linux clears response_length. */
	TEST_RES(read(fd, response, 0), _ret == 0);

	TEST_RES(write(fd, tpm_get_random_command,
		       sizeof(tpm_get_random_command)),
		 _ret == sizeof(tpm_get_random_command));
	TEST_RES(read(fd, response, sizeof(response)),
		 _ret >= TPM_HEADER_SIZE + 2);

	TEST_SUCC(close(fd));
}
END_TEST()

/* Exercise common read-only commands and their variable-length responses. */
FN_TEST(tpm_gets_capability_and_reads_pcr)
{
	uint8_t response[512] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	len = TEST_RES(tpm_transact_sync(fd, tpm_get_capability_command,
					 sizeof(tpm_get_capability_command),
					 response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 17);

	if (len >= TPM_HEADER_SIZE + 17) {
		TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)len);
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
		TEST_RES(tpm_read_be32(response + 11), _ret == 0x00000006);
		TEST_RES(tpm_read_be32(response + 15), _ret >= 1);
		TEST_RES(tpm_read_be32(response + 19), _ret == 0x00000105);
		TEST_RES(tpm_read_be32(response + 23), _ret != 0);
	}

	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(fd, tpm_pcr_read_command,
					 sizeof(tpm_pcr_read_command), response,
					 sizeof(response)),
		       _ret > TPM_HEADER_SIZE);

	if (len > TPM_HEADER_SIZE) {
		TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)len);
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

/* Neither a TPM error response nor char-layer EINVAL may poison later commands. */
FN_TEST(tpm_preserves_error_responses_and_recovers)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	TEST_RES(write(fd, tpm_unknown_command, sizeof(tpm_unknown_command)),
		 _ret == sizeof(tpm_unknown_command));

	len = TEST_RES(read(fd, response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE);
	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_ERRNO(write(fd, tpm_invalid_size_command,
			 sizeof(tpm_invalid_size_command)),
		   EINVAL);

	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(fd, tpm_get_random_command,
					 sizeof(tpm_get_random_command),
					 response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* GetRandom may return fewer bytes, but all reported lengths must be coherent. */
FN_TEST(tpm_random_lengths_and_data_sanity)
{
	static const uint16_t sizes[] = { 1, 32, 64 };
	uint8_t response[256] = { 0 };
	uint8_t command[12];
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		ssize_t len;
		uint16_t returned;

		memset(response, 0, sizeof(response));
		tpm_build_get_random_command(command, sizes[i]);

		len = TEST_RES(tpm_transact_sync(fd, command, sizeof(command),
						 response, sizeof(response)),
			       _ret >= TPM_HEADER_SIZE + 2);

		if (len < TPM_HEADER_SIZE + 2)
			continue;

		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
		returned = tpm_read_be16(response + TPM_HEADER_SIZE);
		TEST_RES(returned, _ret > 0 && _ret <= sizes[i]);
		TEST_RES(tpm_read_be32(response + 2), _ret == (uint32_t)len);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

/* Save, reload, and flush a session to cover raw context-message round trips. */
FN_TEST(tpm_context_save_load_round_trip)
{
	uint8_t response[TPM_BUFSIZE] = { 0 };
	uint8_t command[TPM_BUFSIZE] = { 0 };
	uint8_t flush_command[14];
	uint32_t session_handle;
	ssize_t response_len;
	int fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));

	response_len = TEST_RES(
		tpm_transact_sync(fd, tpm_start_auth_session_command,
				  sizeof(tpm_start_auth_session_command),
				  response, sizeof(response)),
		_ret >= TPM_HEADER_SIZE + 4);
	if (response_len < TPM_HEADER_SIZE + 4) {
		TEST_SUCC(close(fd));
		return;
	}

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	session_handle = tpm_read_be32(response + TPM_HEADER_SIZE);

	tpm_build_command_header(command, 14, TPM2_CC_CONTEXT_SAVE);
	command[10] = (uint8_t)(session_handle >> 24);
	command[11] = (uint8_t)(session_handle >> 16);
	command[12] = (uint8_t)(session_handle >> 8);
	command[13] = (uint8_t)session_handle;

	response_len = TEST_RES(tpm_transact_sync(fd, command, 14, response,
						  sizeof(response)),
				_ret > TPM_HEADER_SIZE);
	if (response_len <= TPM_HEADER_SIZE) {
		TEST_SUCC(close(fd));
		return;
	}

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	if ((size_t)response_len > sizeof(command)) {
		TEST_RES(response_len, _ret <= (ssize_t)sizeof(command));
		TEST_SUCC(close(fd));
		return;
	}

	tpm_build_command_header(command, (size_t)response_len,
				 TPM2_CC_CONTEXT_LOAD);
	memmove(command + TPM_HEADER_SIZE, response + TPM_HEADER_SIZE,
		(size_t)response_len - TPM_HEADER_SIZE);

	response_len =
		TEST_RES(tpm_transact_sync(fd, command, (size_t)response_len,
					   response, sizeof(response)),
			 _ret >= TPM_HEADER_SIZE + 4);
	if (response_len < TPM_HEADER_SIZE + 4) {
		TEST_SUCC(close(fd));
		return;
	}

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	session_handle = tpm_read_be32(response + TPM_HEADER_SIZE);

	tpm_build_flush_context_command(flush_command, session_handle);
	response_len = TEST_RES(tpm_transact_sync(fd, flush_command,
						  sizeof(flush_command),
						  response, sizeof(response)),
				_ret == TPM_HEADER_SIZE);
	if (response_len == TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	response_len = TEST_RES(tpm_transact_sync(fd, flush_command,
						  sizeof(flush_command),
						  response, sizeof(response)),
				_ret >= TPM_HEADER_SIZE);
	if (response_len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()
