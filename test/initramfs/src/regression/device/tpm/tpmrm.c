// SPDX-License-Identifier: MPL-2.0

/*
 * Linux-compatible resource-manager regression tests for /dev/tpmrm0.
 *
 * Each independent open owns a resource space. These tests cover virtual-handle
 * isolation, capability filtering, context swap-in/swap-out, recovery after
 * capacity failures, and close cleanup.
 */

#include <stdbool.h>

#include "tpm_test_common.h"

FN_SETUP(check_tpm_availability)
{
	tpm_require_device(TPM_DEVICE);
	tpm_require_device(TPMRM_DEVICE);
}
END_SETUP()

/* /dev/tpmrm0 must be registered as a character device. */
FN_TEST(tpmrm_device_is_character_device)
{
	struct stat stat_buf;

	TEST_RES(stat(TPMRM_DEVICE, &stat_buf), S_ISCHR(stat_buf.st_mode));
}
END_TEST()

/* Unlike the raw device, the RM permits multiple independent open spaces. */
FN_TEST(tpmrm_allows_multiple_independent_opens)
{
	int fd1 = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));
	int fd2 = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	TEST_SUCC(close(fd2));
	TEST_SUCC(close(fd1));
}
END_TEST()

/* Known commands pass the CC table; unknown ones get a resmgr-layer TPM RC. */
FN_TEST(tpmrm_cc_table_accepts_known_and_rejects_unknown_commands)
{
	uint8_t response[256] = { 0 };
	ssize_t len;
	int fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	/* A known command must be accepted through the RM. */
	len = TEST_RES(tpm_transact_sync(fd, tpm_get_random_command,
					 sizeof(tpm_get_random_command),
					 response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 2);
	if (len >= TPM_HEADER_SIZE + 2)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	/* Linux synthesizes a resource-manager-layer TPM command-code response. */
	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(fd, tpm_unknown_command,
					 sizeof(tpm_unknown_command), response,
					 sizeof(response)),
		       _ret == TPM_HEADER_SIZE);
	if (len == TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6),
			 _ret == (TPM2_RC_COMMAND_CODE |
				  TSS2_RESMGR_TPM_RC_LAYER));

	TEST_SUCC(close(fd));
}
END_TEST()

/* Check Linux behavior for flushing a virtual session across RM spaces. */
FN_TEST(tpmrm_session_flush_across_spaces)
{
	uint8_t response[512] = { 0 };
	uint8_t flush_command[14];
	uint32_t session_handle;
	ssize_t len;
	int owner_fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));
	int other_fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	len = TEST_RES(tpm_transact_sync(owner_fd,
					 tpm_start_auth_session_command,
					 sizeof(tpm_start_auth_session_command),
					 response, sizeof(response)),
		       _ret >= TPM_HEADER_SIZE + 4);

	if (len < TPM_HEADER_SIZE + 4)
		goto out;

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	session_handle = tpm_read_be32(response + TPM_HEADER_SIZE);
	TEST_RES(session_handle >> 24, _ret == 0x02 || _ret == 0x03);

	tpm_build_flush_context_command(flush_command, session_handle);

	/*
	 * Observed Linux behavior: another RM fd can issue FlushContext using
	 * this session handle, and the TPM accepts the flush.
	 */
	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(other_fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret == TPM_HEADER_SIZE);

	if (len == TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

	/*
	 * The original owner now holds a stale session handle. Linux still
	 * completes the write/read transaction and returns a TPM error
	 * response rather than failing write() with a host errno.
	 */
	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(owner_fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret >= TPM_HEADER_SIZE);

	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

out:
	TEST_SUCC(close(other_fd));
	TEST_SUCC(close(owner_fd));
}
END_TEST()

/* Closing an RM fd must synchronously destroy the sessions in its space. */
FN_TEST(tpmrm_close_discards_session_space)
{
	uint8_t response[512] = { 0 };
	uint8_t flush_command[14];
	uint32_t session_handle;
	ssize_t len;
	int fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

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
	 * tpmrm close destroys that space. The raw device should no longer be
	 * able to flush the session because it was already removed.
	 */
	fd = TEST_SUCC(open(TPM_DEVICE, O_RDWR));
	tpm_build_flush_context_command(flush_command, session_handle);

	len = TEST_RES(tpm_transact_sync(fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret >= TPM_HEADER_SIZE);

	if (len >= TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* Transient handles and capability results are visible only to their owner. */
FN_TEST(tpmrm_virtualizes_objects_and_filters_capabilities)
{
	uint8_t response[1024] = { 0 };
	uint8_t flush_command[14];
	uint32_t object_handle;
	uint32_t owner_count;
	ssize_t len;
	bool found = false;
	int owner_fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));
	int other_fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	len = TEST_RES(
		tpm_transact_sync(owner_fd, tpm_hash_sequence_start_command,
				  sizeof(tpm_hash_sequence_start_command),
				  response, sizeof(response)),
		_ret >= TPM_HEADER_SIZE + 4);

	if (len < TPM_HEADER_SIZE + 4)
		goto out;

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	object_handle = tpm_read_be32(response + TPM_HEADER_SIZE);
	TEST_RES(object_handle >> 24, _ret == 0x80);

	/* The owner sees its own virtual transient handle. */
	memset(response, 0, sizeof(response));
	len = TEST_RES(
		tpm_transact_sync(owner_fd, tpm_get_transient_handles_command,
				  sizeof(tpm_get_transient_handles_command),
				  response, sizeof(response)),
		_ret >= 19);

	if (len < 19)
		goto out;

	TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	owner_count = tpm_read_be32(response + 15);
	TEST_RES(owner_count,
		 _ret >= 1 && _ret <= (uint32_t)(((size_t)len - 19) / 4));

	if (owner_count <= (uint32_t)(((size_t)len - 19) / 4)) {
		for (uint32_t i = 0; i < owner_count; i++)
			found |= tpm_read_be32(response + 19 + i * 4) ==
				 object_handle;
		TEST_RES(found, _ret == true);
	}

	/* Another RM space must not see the owner's transient handle. */
	memset(response, 0, sizeof(response));
	len = TEST_RES(
		tpm_transact_sync(other_fd, tpm_get_transient_handles_command,
				  sizeof(tpm_get_transient_handles_command),
				  response, sizeof(response)),
		_ret >= 19);

	if (len >= 19) {
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
		TEST_RES(tpm_read_be32(response + 15), _ret == 0);
	}

	tpm_build_flush_context_command(flush_command, object_handle);

	/*
	 * Linux tpm2_map_command() rejects a transient virtual handle that
	 * does not belong to the current RM space with EINVAL.
	 */
	TEST_ERRNO(write(other_fd, flush_command, sizeof(flush_command)),
		   EINVAL);

	/* The owner can still restore/use/flush its own virtual handle. */
	memset(response, 0, sizeof(response));
	len = TEST_RES(tpm_transact_sync(owner_fd, flush_command,
					 sizeof(flush_command), response,
					 sizeof(response)),
		       _ret == TPM_HEADER_SIZE);

	if (len == TPM_HEADER_SIZE)
		TEST_RES(tpm_read_be32(response + 6), _ret == 0);

out:
	TEST_SUCC(close(other_fd));
	TEST_SUCC(close(owner_fd));
}
END_TEST()

/* Reach transient-object capacity and verify existing objects remain flushable. */
FN_TEST(tpmrm_transient_object_capacity_and_cleanup)
{
	enum { MAX_ATTEMPTS = 8 };
	uint8_t response[1024] = { 0 };
	uint8_t flush_command[14];
	uint32_t handles[MAX_ATTEMPTS] = { 0 };
	size_t created = 0;
	int fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	/*
	 * Do not hard-code "the fourth command must return ENOMEM".
	 * The effective stopping point can depend on the kernel version and
	 * the TPM backend/resource state. Instead, keep creating until the
	 * first host error or TPM error response, then verify that all
	 * previously returned virtual handles are still usable and cleanable.
	 */
	for (size_t i = 0; i < MAX_ATTEMPTS; i++) {
		ssize_t written;
		ssize_t len;
		uint32_t rc;

		errno = 0;
		written = write(fd, tpm_hash_sequence_start_command,
				sizeof(tpm_hash_sequence_start_command));

		if (written < 0)
			break;

		TEST_RES(written,
			 _ret == sizeof(tpm_hash_sequence_start_command));

		memset(response, 0, sizeof(response));
		len = TEST_RES(read(fd, response, sizeof(response)),
			       _ret >= TPM_HEADER_SIZE);

		if (len < TPM_HEADER_SIZE)
			break;

		rc = tpm_read_be32(response + 6);
		if (rc != 0)
			break;

		if (len < TPM_HEADER_SIZE + 4) {
			TEST_RES(len, _ret >= TPM_HEADER_SIZE + 4);
			break;
		}

		handles[created] = tpm_read_be32(response + TPM_HEADER_SIZE);
		TEST_RES(handles[created] >> 24, _ret == 0x80);
		created++;
	}

	TEST_RES(created, _ret >= 1);

	/*
	 * A capacity/resource-limit failure must not corrupt the virtual
	 * handles that were successfully created before it.
	 */
	for (size_t i = 0; i < created; i++) {
		ssize_t len;

		tpm_build_flush_context_command(flush_command, handles[i]);
		memset(response, 0, sizeof(response));

		len = TEST_RES(tpm_transact_sync(fd, flush_command,
						 sizeof(flush_command),
						 response, sizeof(response)),
			       _ret == TPM_HEADER_SIZE);

		if (len == TPM_HEADER_SIZE)
			TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

/* Session-capacity failure must preserve virtual sessions created beforehand. */
FN_TEST(tpmrm_session_capacity_failure_preserves_existing_sessions)
{
	enum { MAX_ATTEMPTS = 8 };
	uint8_t response[1024] = { 0 };
	uint8_t flush_command[14];
	uint32_t handles[MAX_ATTEMPTS] = { 0 };
	size_t created = 0;
	int fd = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));

	/*
	 * Session capacity depends on the TPM/backend state. Probe until the
	 * first host error or TPM error response instead of assuming that
	 * exactly three sessions must always be creatable.
	 */
	for (size_t i = 0; i < MAX_ATTEMPTS; i++) {
		ssize_t written;
		ssize_t len;
		uint32_t rc;

		errno = 0;
		written = write(fd, tpm_start_auth_session_command,
				sizeof(tpm_start_auth_session_command));

		if (written < 0)
			break;

		TEST_RES(written,
			 _ret == sizeof(tpm_start_auth_session_command));

		memset(response, 0, sizeof(response));
		len = TEST_RES(read(fd, response, sizeof(response)),
			       _ret >= TPM_HEADER_SIZE);

		if (len < TPM_HEADER_SIZE)
			break;

		rc = tpm_read_be32(response + 6);
		if (rc != 0)
			break;

		if (len < TPM_HEADER_SIZE + 4) {
			TEST_RES(len, _ret >= TPM_HEADER_SIZE + 4);
			break;
		}

		handles[created] = tpm_read_be32(response + TPM_HEADER_SIZE);
		TEST_RES(handles[created] >> 24, _ret == 0x02 || _ret == 0x03);
		created++;
	}

	TEST_RES(created, _ret >= 1);

	/*
	 * The important regression property is recovery: reaching a resource
	 * or session limit must not invalidate sessions that were previously
	 * created successfully.
	 */
	for (size_t i = 0; i < created; i++) {
		ssize_t len;

		tpm_build_flush_context_command(flush_command, handles[i]);
		memset(response, 0, sizeof(response));

		len = TEST_RES(tpm_transact_sync(fd, flush_command,
						 sizeof(flush_command),
						 response, sizeof(response)),
			       _ret == TPM_HEADER_SIZE);

		if (len == TPM_HEADER_SIZE)
			TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

/* Alternate among spaces to force object ContextSave/ContextLoad cycles. */
FN_TEST(tpmrm_saves_and_restores_objects_across_spaces)
{
	enum { NR_SPACES = 4 };
	int fds[NR_SPACES];
	uint32_t handles[NR_SPACES] = { 0 };
	uint8_t response[1024] = { 0 };
	uint8_t flush_command[14];
	size_t opened = 0;
	size_t created = 0;

	for (size_t i = 0; i < NR_SPACES; i++) {
		fds[i] = TEST_SUCC(open(TPMRM_DEVICE, O_RDWR));
		opened++;
	}

	/*
	 * Each independent RM space creates one transient object. Switching
	 * between spaces exercises save/restore of the per-space contexts.
	 */
	for (size_t i = 0; i < NR_SPACES; i++) {
		ssize_t len;

		memset(response, 0, sizeof(response));
		len = TEST_RES(tpm_transact_sync(
				       fds[i], tpm_hash_sequence_start_command,
				       sizeof(tpm_hash_sequence_start_command),
				       response, sizeof(response)),
			       _ret >= TPM_HEADER_SIZE + 4);

		if (len < TPM_HEADER_SIZE + 4)
			goto out;

		TEST_RES(tpm_read_be32(response + 6), _ret == 0);
		handles[i] = tpm_read_be32(response + TPM_HEADER_SIZE);
		TEST_RES(handles[i] >> 24, _ret == 0x80);
		created++;
	}

	/* Re-enter each space and use its saved virtual handle. */
	for (size_t i = 0; i < created; i++) {
		ssize_t len;

		tpm_build_flush_context_command(flush_command, handles[i]);
		memset(response, 0, sizeof(response));

		len = TEST_RES(tpm_transact_sync(fds[i], flush_command,
						 sizeof(flush_command),
						 response, sizeof(response)),
			       _ret == TPM_HEADER_SIZE);

		if (len == TPM_HEADER_SIZE)
			TEST_RES(tpm_read_be32(response + 6), _ret == 0);
	}

out:
	for (size_t i = 0; i < opened; i++)
		TEST_SUCC(close(fds[i]));
}
END_TEST()