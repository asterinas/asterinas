// SPDX-License-Identifier: MPL-2.0

#define SOCK_TYPE SOCK_STREAM
#include "unix_streamlike_prologue.h"

FN_TEST(sendto)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(sendto(sk_unbound, buf, 1, 0, &LISTEN_ADDR, LISTEN_ADDRLEN),
		   EOPNOTSUPP);
	TEST_ERRNO(sendto(sk_bound, buf, 1, 0, &LISTEN_ADDR2, LISTEN_ADDRLEN2),
		   EOPNOTSUPP);
	TEST_ERRNO(sendto(sk_listen, buf, 1, 0, &BOUND_ADDR, BOUND_ADDRLEN),
		   EOPNOTSUPP);
	TEST_ERRNO(sendto(sk_accepted, buf, 1, 0, &UNNAMED_ADDR,
			  UNNAMED_ADDRLEN),
		   EISCONN);
}
END_TEST()

FN_TEST(scm_rights)
{
	int fildes[2];
	char buf[20] = "abcdefg";
	char cbuf[CMSG_SPACE(sizeof(int) * 3)];
	struct iovec iov;
	struct msghdr mhdr;
	struct cmsghdr *chdr;
	int *cdata;
	int cfds[2];

	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0, fildes));

	memset(&mhdr, 0, sizeof(mhdr));
	mhdr.msg_iov = &iov;
	mhdr.msg_iovlen = 1;
	mhdr.msg_control = cbuf;
	mhdr.msg_controllen = CMSG_SPACE(sizeof(int) * 3);

	iov.iov_base = buf;
	iov.iov_len = 1;

	chdr = CMSG_FIRSTHDR(&mhdr);
	chdr->cmsg_level = SOL_SOCKET;
	chdr->cmsg_type = SCM_RIGHTS;
	chdr->cmsg_len = CMSG_SPACE(sizeof(int) * 3);

	cdata = (int *)CMSG_DATA(chdr);
	TEST_SUCC(pipe(cfds));
	cdata[0] = cfds[0];
	cdata[1] = cfds[0];
	cdata[2] = cfds[1];

	// Sending control messages with zero bytes to a stream socket
	// seems to "succeed". However, no data or control messages can
	// be transmitted.
	mhdr.msg_iovlen = 0;
	TEST_SUCC(sendmsg(fildes[0], &mhdr, 0));
	mhdr.msg_iovlen = 1;

	// > (1)  sendmsg(2) of four bytes, with no ancillary data.
	// > (2)  sendmsg(2) of one byte, with ancillary data.
	// > (3)  sendmsg(2) of four bytes, with no ancillary data.
	//  -- https://man7.org/linux/man-pages/man7/unix.7.html
	TEST_RES(send(fildes[0], buf, 4, 0), _ret == 4);
	TEST_RES(sendmsg(fildes[0], &mhdr, 0), _ret == 1);
	TEST_RES(send(fildes[0], buf, 4, 0), _ret == 4);

	memset(&mhdr, 0, sizeof(mhdr));
	mhdr.msg_iov = &iov;
	mhdr.msg_iovlen = 1;
	mhdr.msg_control = cbuf;
	mhdr.msg_controllen = CMSG_SPACE(sizeof(int));

	iov.iov_base = buf;
	iov.iov_len = sizeof(buf);

	memset(cbuf, 0, sizeof(cbuf));

	// > Suppose that the receiver now performs recvmsg(2) calls each with
	// > a buffer size of 20 bytes.  The first call will receive five bytes
	// > of data, along with the ancillary data sent by the second
	// > sendmsg(2) call.
	TEST_RES(recvmsg(fildes[1], &mhdr, 0),
		 _ret == 5 &&
			 mhdr.msg_controllen == CMSG_SPACE(sizeof(int) * 2) &&
			 (chdr = CMSG_FIRSTHDR(&mhdr)) &&
			 chdr->cmsg_level == SOL_SOCKET &&
			 chdr->cmsg_type == SCM_RIGHTS &&
			 chdr->cmsg_len == CMSG_SPACE(sizeof(int) * 2) &&
			 (cdata = (int *)CMSG_DATA(chdr)) &&
			 cdata[0] == cfds[1] + 1 && cdata[1] == cfds[1] + 2);
	// > The next call will receive the remaining four
	// > bytes of data.
	TEST_RES(recv(fildes[1], buf, sizeof(buf), 0), _ret == 4);

	// The purpose of the tests below is to verify that the received file
	// descriptors are functional.
	TEST_RES(write(cfds[1], "x", 1), _ret == 1);
	TEST_RES(read(cdata[0], buf, 1), _ret == 1 && buf[0] == 'x');
	TEST_RES(write(cfds[1], "y", 1), _ret == 1);
	TEST_RES(read(cdata[1], buf, 1), _ret == 1 && buf[0] == 'y');

	TEST_SUCC(close(cdata[0]));
	TEST_SUCC(close(cdata[1]));
	TEST_SUCC(close(cfds[0]));

	TEST_ERRNO(write(cfds[1], "y", 1), EPIPE);
	TEST_SUCC(close(cfds[1]));

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()

#include "unix_streamlike_epilogue.h"
