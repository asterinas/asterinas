// SPDX-License-Identifier: MPL-2.0

#ifndef NETLINK_SOCKOPTION_H
#define NETLINK_SOCKOPTION_H

#include <linux/netlink.h>
#include <sys/socket.h>

#include "../common/test.h"

// Use these macros inside FN_TEST; the getter exposes the returned value as _opt.
#define TEST_GETSOCKOPT_INT(fd, option, cond)                                  \
	do {                                                                   \
		int _opt = -1;                                                 \
		socklen_t _optlen = sizeof(_opt);                              \
		TEST_RES(getsockopt(fd, SOL_NETLINK, option, &_opt, &_optlen), \
			 _optlen == sizeof(_opt) && (cond));                   \
	} while (0)

#define TEST_SETSOCKOPT_INT(fd, option, value)                       \
	do {                                                         \
		int _opt = (value);                                  \
		TEST_SUCC(setsockopt(fd, SOL_NETLINK, option, &_opt, \
				     sizeof(_opt)));                 \
	} while (0)

#endif
