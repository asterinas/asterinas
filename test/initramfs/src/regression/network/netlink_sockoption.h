// SPDX-License-Identifier: MPL-2.0

#ifndef NETLINK_SOCKOPTION_H
#define NETLINK_SOCKOPTION_H

#include <linux/netlink.h>
#include <sys/socket.h>

#include "../common/test.h"

#define TEST_SETSOCKOPT_INT(fd, option, value)                       \
	do {                                                         \
		int _opt = (value);                                  \
		TEST_SUCC(setsockopt(fd, SOL_NETLINK, option, &_opt, \
				     sizeof(_opt)));                 \
	} while (0)

#endif
