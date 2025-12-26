// SPDX-License-Identifier: MPL-2.0

FN_SETUP(cleanup)
{
	CHECK(close(pipe_1[0]));
	CHECK(close(pipe_1[1]));
	CHECK(close(pipe_2[0]));
	CHECK(close(pipe_2[1]));
	CHECK(close(sock[0]));
	CHECK(close(sock[1]));
	CHECK(close(epoll_fd));
	CHECK(close(event_fd));
	CHECK(close(timer_fd));
	CHECK(close(signal_fd));
	CHECK(close(inotify_fd));
	CHECK(close(pid_fd));
	CHECK(close(mem_fd));
	CHECK(kill(child, SIGKILL));
	CHECK(waitpid(child, NULL, 0));
}
END_SETUP()