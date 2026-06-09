/* SPDX-License-Identifier: MPL-2.0 */

#define TEST_NAME(name) TEST_NAME1(PREFIX, name)
#define TEST_NAME1(prefix, name) TEST_NAME2(prefix, name)
#define TEST_NAME2(prefix, name) prefix##name
#define DEFINE_TEST(name) DEFINE_TEST1(TEST_NAME(name))
#define DEFINE_TEST1(name) FN_TEST(name)

#define FDS()        \
	int send_fd; \
	int recv_fd

#define SEND(offset, len)                                     \
	TEST_RES(send(send_fd, PAYLOAD + (offset), (len), 0), \
		 _ret == (ssize_t)(len))

#define PEEK(offset, len)                                                      \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		msg_flags = 0;                                                 \
		TEST_RES(peek_message(recv_fd, buf, (len), &msg_flags),        \
			 _ret == (ssize_t)(len) &&                             \
				 (msg_flags & MSG_TRUNC) == 0 &&               \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
	} while (0)

#define RECV(offset, len)                                                      \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		TEST_RES(recv(recv_fd, buf, (len), 0),                         \
			 _ret == (ssize_t)(len) &&                             \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
	} while (0)

#define CLOSE()                            \
	do {                               \
		TEST_SUCC(close(send_fd)); \
		TEST_SUCC(close(recv_fd)); \
	} while (0)

DEFINE_TEST(short_peek_short_reads)
{
	FDS();
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	CONNECT();

	SEND(0, PAYLOAD_LEN);
	PEEK(0, SHORT_LEN);
	RECV(0, SHORT_LEN);
	RECV(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);

	CLOSE();
}
END_TEST()

DEFINE_TEST(short_peek_full_read)
{
	FDS();
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	CONNECT();

	SEND(0, PAYLOAD_LEN);
	PEEK(0, SHORT_LEN);
	RECV(0, PAYLOAD_LEN);

	CLOSE();
}
END_TEST()

DEFINE_TEST(full_peek_short_reads)
{
	FDS();
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	CONNECT();

	SEND(0, PAYLOAD_LEN);
	PEEK(0, PAYLOAD_LEN);
	RECV(0, SHORT_LEN);
	RECV(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);

	CLOSE();
}
END_TEST()

DEFINE_TEST(append_after_peek_short_reads)
{
	FDS();
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	CONNECT();

	SEND(0, SHORT_LEN);
	PEEK(0, SHORT_LEN);

	SEND(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);
	WAIT_APPENDED_READABLE();
	RECV(0, SHORT_LEN);
	RECV(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);

	CLOSE();
}
END_TEST()

DEFINE_TEST(append_after_peek_full_read)
{
	FDS();
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	CONNECT();

	SEND(0, SHORT_LEN);
	PEEK(0, SHORT_LEN);

	SEND(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);
	WAIT_APPENDED_READABLE();
	RECV(0, PAYLOAD_LEN);

	CLOSE();
}
END_TEST()

#undef TEST_NAME
#undef TEST_NAME1
#undef TEST_NAME2
#undef DEFINE_TEST
#undef DEFINE_TEST1
#undef FDS
#undef SEND
#undef PEEK
#undef RECV
#undef CLOSE
