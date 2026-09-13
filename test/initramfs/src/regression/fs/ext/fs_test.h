// SPDX-License-Identifier: MPL-2.0

#ifndef EXT_FS_TEST_H
#define EXT_FS_TEST_H

#include <stddef.h>

#define EXT_TEST_ROOT "/ext-test"

/* Check a complete zero-filled hole or repeated-byte test pattern. */
static inline int buffer_is_filled(const char *buffer, size_t len,
				   unsigned char value)
{
	for (size_t i = 0; i < len; i++)
		if ((unsigned char)buffer[i] != value)
			return 0;
	return 1;
}

#endif
