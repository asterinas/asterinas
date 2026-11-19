// SPDX-License-Identifier: MPL-2.0

#ifndef TPM_TEST_COMMON_H
#define TPM_TEST_COMMON_H

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../../common/test.h"

/* Linux TPM misc-device ABI constants and the fixed char-layer buffer limit. */
#define TPM_DEVICE "/dev/tpm0"
#define TPMRM_DEVICE "/dev/tpmrm0"
#define TPM_MAJOR 10
#define TPM_MINOR 224
#define TPM_BUFSIZE 4096
#define TPM_HEADER_SIZE 10
#define TPM_ST_NO_SESSIONS 0x8001U
#define TPM2_RC_COMMAND_CODE 0x0143U
#define TSS2_RESMGR_TPM_RC_LAYER (11U << 16)
#define TPM2_CC_CONTEXT_LOAD 0x00000161U
#define TPM2_CC_CONTEXT_SAVE 0x00000162U
#define TPM2_CC_FLUSH_CONTEXT 0x00000165U
#define TPM2_CC_START_AUTH_SESSION 0x00000176U
#define TPM2_CC_GET_CAPABILITY 0x0000017aU
#define TPM2_CC_GET_RANDOM 0x0000017bU
#define TPM2_CC_PCR_READ 0x0000017eU
#define TPM2_CC_HASH_SEQUENCE_START 0x00000186U

/*
 * Each array below is a complete big-endian TPM2 wire message for write(2):
 * [0..2) tag, [2..6) total size, [6..10) command code, then parameters.
 * Fixed byte arrays keep these tests independent of the userspace tpm2-tss.
 */
static const uint8_t tpm_get_random_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x01, 0x7b, 0x00, 0x20,
};

static const uint8_t tpm_get_capability_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x01, 0x7a, 0x00,
	0x00, 0x00, 0x06, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00, 0x00, 0x01,
};

static const uint8_t tpm_pcr_read_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x01, 0x7e,
	0x00, 0x00, 0x00, 0x01, 0x00, 0x0b, 0x03, 0x01, 0x00, 0x00,
};

static const uint8_t tpm_unknown_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x0a, 0xff, 0xff, 0xff, 0xff,
};

static const uint8_t tpm_invalid_size_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x01, 0x7b,
};

static const uint8_t tpm_invalid_tag_command[] = {
	0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x01, 0x7b,
};

static const uint8_t tpm_start_auth_session_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x01, 0x76, 0x40,
	0x00, 0x00, 0x07, 0x40, 0x00, 0x00, 0x07, 0x00, 0x10, 0x00, 0x01,
	0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
	0x0d, 0x0e, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x0b,
};

static const uint8_t tpm_hash_sequence_start_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x0e, 0x00,
	0x00, 0x01, 0x86, 0x00, 0x00, 0x00, 0x0b,
};

static const uint8_t tpm_get_transient_handles_command[] = {
	0x80, 0x01, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x01, 0x7a, 0x00,
	0x00, 0x00, 0x01, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40,
};

static inline uint16_t tpm_read_be16(const uint8_t *bytes)
{
	return ((uint16_t)bytes[0] << 8) | bytes[1];
}

static inline uint32_t tpm_read_be32(const uint8_t *bytes)
{
	return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
	       ((uint32_t)bytes[2] << 8) | bytes[3];
}

/* Build only the common ten-byte, no-session command header. */
static inline void tpm_build_command_header(uint8_t *command, size_t size,
					    uint32_t code)
{
	command[0] = 0x80;
	command[1] = 0x01;
	command[2] = (uint8_t)(size >> 24);
	command[3] = (uint8_t)(size >> 16);
	command[4] = (uint8_t)(size >> 8);
	command[5] = (uint8_t)size;
	command[6] = (uint8_t)(code >> 24);
	command[7] = (uint8_t)(code >> 16);
	command[8] = (uint8_t)(code >> 8);
	command[9] = (uint8_t)code;
}

static inline void tpm_build_get_random_command(uint8_t command[12],
						uint16_t size)
{
	tpm_build_command_header(command, 12, TPM2_CC_GET_RANDOM);
	command[10] = (uint8_t)(size >> 8);
	command[11] = (uint8_t)size;
}

static inline void tpm_build_flush_context_command(uint8_t command[14],
						   uint32_t handle)
{
	tpm_build_command_header(command, 14, TPM2_CC_FLUSH_CONTEXT);
	command[10] = (uint8_t)(handle >> 24);
	command[11] = (uint8_t)(handle >> 16);
	command[12] = (uint8_t)(handle >> 8);
	command[13] = (uint8_t)handle;
}

/* A missing device means unsupported test setup, not a driver failure. */
static inline void tpm_require_device(const char *path)
{
	if (access(path, F_OK) == 0)
		return;
	if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
		fprintf(stderr, "TPM tests skipped: %s is unavailable\n", path);
		exit(EXIT_SUCCESS);
	}
	perror("failed to inspect TPM device");
	exit(EXIT_FAILURE);
}

/* A synchronous transaction is one complete write followed by one read. */
static inline ssize_t tpm_transact_sync(int fd, const uint8_t *command,
					size_t command_len, uint8_t *response,
					size_t response_size)
{
	ssize_t ret = write(fd, command, command_len);
	if (ret < 0)
		return -1;
	if (ret != (ssize_t)command_len) {
		errno = EIO;
		return -1;
	}
	return read(fd, response, response_size);
}

/* Wait for POLLIN before reading a response or errno produced by async work. */
static inline ssize_t tpm_wait_and_read_response(int fd, uint8_t *response,
						 size_t response_size,
						 int timeout_ms)
{
	struct pollfd pfd = { .fd = fd, .events = POLLIN };
	int ret;
	do {
		ret = poll(&pfd, 1, timeout_ms);
	} while (ret < 0 && errno == EINTR);
	if (ret == 0) {
		errno = ETIMEDOUT;
		return -1;
	}
	if (ret < 0)
		return -1;
	if (!(pfd.revents & POLLIN)) {
		errno = EIO;
		return -1;
	}
	return read(fd, response, response_size);
}

#endif