// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <unistd.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/file.h>
#include <poll.h>

#include "../../common/test.h"

#define FILENAME "/tmp/testfile"
#define DIRNAME "/tmp"
#define PAGE_SIZE 4096

static struct flock f_rdlck = {
	.l_type = F_RDLCK,
	.l_whence = SEEK_SET,
	.l_start = 0,
	.l_len = 1024,
};
static struct flock f_wrlck = {
	.l_type = F_WRLCK,
	.l_whence = SEEK_SET,
	.l_start = 0,
	.l_len = 1024,
};
static struct flock f_unlck = {
	.l_type = F_UNLCK,
	.l_whence = SEEK_SET,
	.l_start = 0,
	.l_len = 1024,
};

FN_SETUP(create)
{
	CHECK(creat(FILENAME, 0666));
}
END_SETUP()

FN_TEST(readable)
{
	int fd;
	char buf[1];
	void *addr;
	struct pollfd pfd;

	// Test 1: Normal file

	fd = TEST_SUCC(open(FILENAME, O_RDONLY));

	TEST_SUCC(read(fd, buf, sizeof(buf)));
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_ERRNO(ioctl(fd, TCGETS), ENOTTY);
	TEST_ERRNO(ftruncate(fd, 1), EINVAL);
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 1), EBADF);

	addr = TEST_SUCC(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0));
	TEST_SUCC(munmap(addr, PAGE_SIZE));
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EACCES);

	TEST_SUCC(flock(fd, LOCK_SH));
	TEST_SUCC(flock(fd, LOCK_UN));
	TEST_SUCC(flock(fd, LOCK_EX));
	TEST_SUCC(flock(fd, LOCK_UN));

	TEST_SUCC(fcntl(fd, F_SETLK, &f_rdlck));
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_wrlck), EBADF);
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));

	TEST_RES(fcntl(fd, F_GETLK, &f_rdlck), f_rdlck.l_type == F_UNLCK);
	TEST_RES(fcntl(fd, F_GETLK, &f_wrlck), f_wrlck.l_type == F_UNLCK);
	f_rdlck.l_type = F_RDLCK;
	f_wrlck.l_type = F_WRLCK;

	pfd.fd = fd;
	pfd.events = POLLIN | POLLOUT;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));

	TEST_SUCC(close(fd));

	// Test 2: Directory

	fd = TEST_SUCC(open(DIRNAME, O_RDONLY));

	TEST_ERRNO(read(fd, buf, sizeof(buf)), EISDIR);
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_ERRNO(lseek(fd, 0, SEEK_END), EINVAL);
	TEST_ERRNO(ioctl(fd, TCGETS), ENOTTY);
	TEST_ERRNO(ftruncate(fd, 1), EINVAL);
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 1), EBADF);

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0), ENODEV);
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EACCES);

	TEST_SUCC(flock(fd, LOCK_SH));
	TEST_SUCC(flock(fd, LOCK_UN));
	TEST_SUCC(flock(fd, LOCK_EX));
	TEST_SUCC(flock(fd, LOCK_UN));

	TEST_SUCC(fcntl(fd, F_SETLK, &f_rdlck));
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_wrlck), EBADF);
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));

	TEST_RES(fcntl(fd, F_GETLK, &f_rdlck), f_rdlck.l_type == F_UNLCK);
	TEST_RES(fcntl(fd, F_GETLK, &f_wrlck), f_wrlck.l_type == F_UNLCK);
	f_rdlck.l_type = F_RDLCK;
	f_wrlck.l_type = F_WRLCK;

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(writeable)
{
	int fd;
	char buf[1];
	struct pollfd pfd;

	// Test 1: Normal file

	fd = TEST_SUCC(open(FILENAME, O_WRONLY));

	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(write(fd, buf, sizeof(buf)));
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_ERRNO(ioctl(fd, TCGETS), ENOTTY);
	TEST_SUCC(ftruncate(fd, 1));
	TEST_SUCC(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 1));

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0), EACCES);
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EACCES);

	TEST_SUCC(flock(fd, LOCK_SH));
	TEST_SUCC(flock(fd, LOCK_UN));
	TEST_SUCC(flock(fd, LOCK_EX));
	TEST_SUCC(flock(fd, LOCK_UN));

	TEST_ERRNO(fcntl(fd, F_SETLK, &f_rdlck), EBADF);
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));
	TEST_SUCC(fcntl(fd, F_SETLK, &f_wrlck));
	TEST_SUCC(fcntl(fd, F_SETLK, &f_unlck));

	TEST_RES(fcntl(fd, F_GETLK, &f_rdlck), f_rdlck.l_type == F_UNLCK);
	TEST_RES(fcntl(fd, F_GETLK, &f_wrlck), f_wrlck.l_type == F_UNLCK);
	f_rdlck.l_type = F_RDLCK;
	f_wrlck.l_type = F_WRLCK;

	pfd.fd = fd;
	pfd.events = POLLIN | POLLOUT;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));

	TEST_SUCC(close(fd));

	// Test 2: Directory

	TEST_ERRNO(open(DIRNAME, O_WRONLY), EISDIR);
	TEST_ERRNO(open(DIRNAME, O_RDWR), EISDIR);
}
END_TEST()

FN_TEST(path)
{
	int fd;
	char buf[1];
	struct pollfd pfd;

	// Test 1: Normal file

	fd = TEST_SUCC(open(FILENAME, O_RDWR | O_PATH));

	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(lseek(fd, 0, SEEK_SET), EBADF);
	TEST_ERRNO(lseek(fd, 0, SEEK_END), EBADF);
	TEST_ERRNO(ioctl(fd, TCGETS), EBADF);
	TEST_ERRNO(ftruncate(fd, 1), EBADF);
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 1), EBADF);

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0), EBADF);
	// FIXME: Asterinas reports `EACCES` because it performs the permission check first.
#ifdef __asterinas__
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EACCES);
#else
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EBADF);
#endif

	TEST_ERRNO(flock(fd, LOCK_SH), EBADF);
	TEST_ERRNO(flock(fd, LOCK_UN), EBADF);
	TEST_ERRNO(flock(fd, LOCK_EX), EBADF);
	TEST_ERRNO(flock(fd, LOCK_UN), EBADF);

	TEST_ERRNO(fcntl(fd, F_SETLK, &f_rdlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_unlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_wrlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_unlck), EBADF);

	TEST_ERRNO(fcntl(fd, F_GETLK, &f_rdlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_GETLK, &f_wrlck), EBADF);

	pfd.fd = fd;
	pfd.events = POLLIN | POLLOUT;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLNVAL);

	TEST_SUCC(close(fd));

	// Test 2: Directory

	fd = TEST_SUCC(open(DIRNAME, O_RDWR | O_PATH));

	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(lseek(fd, 0, SEEK_SET), EBADF);
	TEST_ERRNO(lseek(fd, 0, SEEK_END), EBADF);
	TEST_ERRNO(ioctl(fd, TCGETS), EBADF);
	TEST_ERRNO(ftruncate(fd, 1), EBADF);
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 1), EBADF);

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED, fd, 0), EBADF);
	// FIXME: Asterinas reports `EACCES` because it performs the permission check first.
#ifdef __asterinas__
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EACCES);
#else
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			0),
		   EBADF);
#endif

	TEST_ERRNO(flock(fd, LOCK_SH), EBADF);
	TEST_ERRNO(flock(fd, LOCK_UN), EBADF);
	TEST_ERRNO(flock(fd, LOCK_EX), EBADF);
	TEST_ERRNO(flock(fd, LOCK_UN), EBADF);

	TEST_ERRNO(fcntl(fd, F_SETLK, &f_rdlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_unlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_wrlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_SETLK, &f_unlck), EBADF);

	TEST_ERRNO(fcntl(fd, F_GETLK, &f_rdlck), EBADF);
	TEST_ERRNO(fcntl(fd, F_GETLK, &f_wrlck), EBADF);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(flags)
{
	int fd;
	char buf[5];

	fd = TEST_SUCC(open(FILENAME, O_WRONLY));
	TEST_RES(write(fd, "hello", 5), _ret == 5);
	TEST_SUCC(close(fd));

	// `O_PATH | O_TRUNC` has no effect.
	fd = TEST_SUCC(open(FILENAME, O_WRONLY | O_PATH | O_TRUNC));
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(FILENAME, O_RDONLY));
	TEST_RES(read(fd, buf, sizeof(buf)),
		 _ret == 5 && memcmp(buf, "hello", 5) == 0);
	TEST_SUCC(close(fd));

	// `O_RDONLY | O_TRUNC` will truncate the file.
	fd = TEST_SUCC(open(FILENAME, O_RDONLY | O_TRUNC));
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == 0);
	TEST_SUCC(close(fd));

	// `O_PATH | O_CREAT` has no effect.
	TEST_ERRNO(open("/tmp/file_does_not_exist", O_PATH | O_CREAT, 0644),
		   ENOENT);
}
END_TEST()

FN_SETUP(unlink)
{
	CHECK(unlink(FILENAME));
}
END_SETUP()
